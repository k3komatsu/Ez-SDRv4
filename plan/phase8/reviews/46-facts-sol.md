## 所見

**修正・補足が必要な点は19件です。** 制限案の採否・優劣は判断していません。

対象は `main 50b5e83` と新規ファイルです。以下、台帳の行番号は `plan/phase8/46-streaming-feature-ledger.md` を指します。

### 1. §3一覧 F1・F8：#70・#72・#71 の帰属が違う

- **問題**：F1 の「#70・#72 の一部」、F8 の「#71」は、その機能が生んだ不具合という整理になっていません。
- **証拠**：
  - [#70](https://github.com/k3komatsu/Ez-SDRv4/issues/70)：一サンプル device burst／EOB の問題。
  - [#72](https://github.com/k3komatsu/Ez-SDRv4/issues/72)：**TX** 周波数更新を untimed continuation が消費しない問題。
  - [#71](https://github.com/k3komatsu/Ez-SDRv4/issues/71)：burst がない間の timed TX retune の蓄積。
  - `plan/phase8/39-dsp-timed-commands.md:74–85,109–115`。
- **訂正**：#70 は §2 の基本的な TX framing と F8、#72 は F2、#71 は F2 の「burst 間にも更新を許す」機能に関係します。#71 は recovery を使わなくても起きます。

### 2. §3一覧 F3：「L1 の chain 走査」を不具合として数える根拠がない

- **問題**：47行の帰属は、実装した検査・試験と、発見された不具合を混同しています。
- **証拠**：
  - `plan/phase8/reviews/l1-test-plan.md:50–53` に並行 restore の chain 走査試験。
  - L1 の実際の指摘一覧は `plan/phase8/reviews/l1-sol.md:5–17`。chain 走査の不具合は列挙されていません。
  - restore に直接関係する実際の不具合は、失敗した flush stop の後にも restore するもの：`plan/phase8/reviews/d9b2-sol.md:9,310–312,345`。
- **訂正**：F3 の確認済み帰属には D9b-2 の restore-after-failed-stop を挙げられます。L1 の走査は「抱える仕組み・試験」であり、引用された記録からは「生んだ不具合」と確認できません。

### 3. §3一覧：関連する履歴が抜けている

- **問題**：現在の一覧だけでは、機能ごとの不具合の量を比較できません。
- **証拠・訂正**：
  - **F1/F2**：live channel がない timed update を後続 segment に渡す設計で、計画レビュー4件・コード後5件。`plan/maintenance/failure-mechanisms.md:38`。
  - **F5**：move destination の開始条件で5件。`plan/maintenance/failure-mechanisms.md:28`。さらに連鎖 move の prune 不具合：`plan/phase8/reviews/1b4e2-sol.md:24–31,114–117`。
  - **F7と基本TXの共有部分**：note 40 の Mock framing 4件と、UHD 内の二重 `TIME_ERROR`。`plan/maintenance/failure-mechanisms.md:48`。
- **訂正**：これらを追加し、「共有」「基本送信にも必要」を区別する必要があります。機能別件数の単純合計はできません。

### 4. §3一覧 F4：「stream-lifecycle-scattered 7件の多く」は内訳と実装先が必要

- **問題**：48行は、UHD の並行処理の不具合と Mock の逐次モデルの不具合をまとめています。
- **証拠**：
  - 7件の集合：`plan/maintenance/failure-mechanisms.md:11`。
  - [#9](https://github.com/k3komatsu/Ez-SDRv4/issues/9)：abort と LateCommand。cold change は不要。
  - [#35](https://github.com/k3komatsu/Ez-SDRv4/issues/35)、[#45](https://github.com/k3komatsu/Ez-SDRv4/issues/45)：Mock の cold change と RX fault。
  - [#46](https://github.com/k3komatsu/Ez-SDRv4/issues/46)、[#47](https://github.com/k3komatsu/Ez-SDRv4/issues/47)：Mock の Stop と cold change。
  - [#50](https://github.com/k3komatsu/Ez-SDRv4/issues/50)：cold change と Provider stop の parity。
  - [#58](https://github.com/k3komatsu/Ez-SDRv4/issues/58)：from-none enable と abort。
- **訂正**：#9 は §2、#58 は F5、#46・#47 は F4/F6共有など、個別に記載すべき事実です。「UHD の競合7件」ではありません。

### 5. F6：#67 は F6 をなくしても残りうる

- **問題**：50行の帰属だけでは、stream 単位の Stop／start_rx をなくせば #67 もなくなるように読めます。
- **証拠**：
  - [#67](https://github.com/k3komatsu/Ez-SDRv4/issues/67) は、先に発行した将来の RX start と **Provider::stop** の系列。
  - `design/18-uhd-radio.md:216,224` は最初の T0 の開始も先行発行し、取消時は origin 後に止める規則。
- **訂正**：F6 は一つの到達経路です。§2で残す最初の timed start と Run 終了にも同じ問題があります。

### 6. F1・F2・F4・F7の根拠：Vision は「class の集合だけ」ではない

- **問題**：57・88行の説明では、関連する Vision の要求が落ちています。
- **証拠**：
  - RUN 中の変更、TX gain、sample-rate change は cold：`design/vision/05-coherence-calibration-and-runtime-semantics.md:124–154`。
  - Session の変更操作：`design/vision/01-purpose-and-core-boundary.md:102–106`。
  - §58 #16 の runtime frequency／rate admission：`design/vision/10-implementation-path-and-testing.md:74`。
  - §58 #9 の動的 TxBurst：同ファイル`:67`。
  - Easy API の周波数・gain・antenna・membership・rate・Stop/start：`design/16-easy-api.md:86–93`。
- **訂正**：Vision は個々の numbered key の class までは指定しませんが、runtime mutation の用途と一部の分類・受入試験は指定しています。一方、**動的 TxBurst の要求だけから、実行中 burst への preemption が必須とは言えません**。§61 の4行動も preemption や cold change を要求してはいません（`design/vision/10-implementation-path-and-testing.md:211–216`）。

### 7. §3全体：RX gain と固定構成の複数stream同時運転が未分類

- **問題**：
  - RX gain は F1(b) で残すと書かれるだけで、独立した利点・負担・制限の帳簿がありません。
  - move／cold change をしない、固定構成の複数stream同時運転も、F1–F8にも§2にも明示されていません。
- **証拠**：
  - RX/TX双方の gain 宣言：`design/07-radio-model.md:269–281`。
  - gain の既存経路：`design/18-uhd-radio.md:266,277`。
  - 宣言streamごとのowner：`design/18-uhd-radio.md:59,216`、`crates/ezsdr-radio-uhd/src/provider/mod.rs:92–109`。
- **訂正**：RX gain は現在の機能です。固定構成の複数stream運転も、F5をなくしてなお残る機能です。full duplex を明示的に対象外にした37行とは別の分類漏れです。

### 8. F3：restore を省くことと、未消費FIFOを残すことが混同されている

- **問題**：82行の因果関係は正しくありません。
- **証拠**：
  - restore は drain／flush と成功した stop **の後**。未消費entryがあれば restoreしない：`design/18-uhd-radio.md:274–276,340–341`。
  - 実装：`crates/ezsdr-radio-uhd/src/provider/rx.rs:321–338,435–458`。
  - note 39 冒頭の訂正：`plan/phase8/39-dsp-timed-commands.md:3`。
  - UHD 4.10：
    - [ddc.v:114–142](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/ddc.v#L114-L142)：残った timed 値は後のsampleでuntimed値を上書きする。
    - [rfnoc_graph.cpp:714–722](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_graph.cpp#L714-L722)、[985–998](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_graph.cpp#L985-L998)：新しいgraphの初期化でblockをflush/reset。
    - [1100–1121](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_graph.cpp#L1100-L1121)：既存graphが生きていれば再利用。
- **訂正**：
  - **VERIFIED**：未消費FIFOを残して同じgraphを使い続ければ、後のsampleで新しいuntimed設定を上書きできます。
  - **VERIFIED**：新しいgraphの構築はflush/resetを行います。
  - **VERIFIED**：restoreだけを省いても、正常にdrainされたentryがFIFOへ戻るわけではありません。失うのは、drain中などに物理的に適用された値を「S時点の値」に戻す処理です。
  - 「次のRun」の結論には、同じDevice/graphを再利用するか、新しく構築するかの条件が必要です。Run境界だけから一律には判定できません。

### 9. F3(b)：release windowをSで止めるだけでは代替にならない

- **問題**：83行は、Sが事前に既知であることと、hand-overがSをまたがないことを暗黙に仮定しています。
- **証拠**：
  - restore対象は `e > S` **または受理がS後**：`design/18-uhd-radio.md:340`。
  - 実装の判定：`crates/ezsdr-radio-uhd/src/provider/control.rs:380–408`。
  - callがSをまたぐ扱い：`design/07-radio-model.md:169–170`。
- **訂正**：任意時刻のSession終了・abortでは、既に渡したcommandを「S前に渡さなかった」ことにはできません。事前既知の終了境界と、進行中callの扱いを別条件として定義しない限り、この段階だけでrestore不要とは言えません。

### 10. F1(c)：segment内だけの受付でもdrain・flush・leftは消えない

- **問題**：64行の断定が強すぎます。
- **反例**：流れるsegmentを覆う `e` のretuneを渡す → 後から `e` より前にStop／Provider stopが来る。
- **証拠**：この系列を扱う規則が `design/18-uhd-radio.md:273–276`。常に残すProvider stopもcutの原因です（`design/07-radio-model.md:168–170`）。
- **訂正**：受付時に既知のcutをまたがせないことと、**hand-over後のcutを禁止すること**は別です。後者を定義しない限り、消費義務と失敗時の残留処理は必要です。

### 11. F2：gainはDUCを通らず、burst間更新を残せばDSP barrierも残る

- **問題**：46・71–74行は、周波数とgainの仕組み・失う機能を一括りにしています。
- **証拠**：
  - gainはradioだけ：`plan/phase8/39-dsp-timed-commands.md:15–17`。
  - UHD [set_rx_gain:1583–1587](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L1583-L1587)、[set_tx_gain:2117–2121](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L2117-L2121)。
  - TX applyとopen/flushのgate：`crates/ezsdr-radio-uhd/src/provider/control.rs:828`、`provider/core.rs:1540–1558`。
  - burst間の値はFIFOに残る：`design/18-uhd-radio.md:287–288`。
- **訂正**：F2を全廃すると失うものにはTX gain変更も含まれます。F2(b)では、開く時／recoveryとの順序制御や未消費DUC値の問題が残り、「流れていない時だけ」では解消しません。

### 12. F4：streamer交換・generation・次segmentの道は単独では消えない

- **問題**：90行の削除対象は他機能と共有しています。
- **証拠**：
  - F6のstart_rxも新しいconfigurationを持つitemを作る：`crates/ezsdr-radio-uhd/src/provider/control.rs:493–509`。
  - F8はstreamerを交換しgenerationを更新：`provider/core.rs:1549–1564`。
  - recovery後もcorrelationを同期：`provider/tx.rs:849–890`。
- **訂正**：F4をなくしても、F6があれば次segment・configure/refusalの道が、F8があればstreamer交換・generation・report同期が残ります。cold固有の部分だけが削除候補です。

### 13. F5：「Owner以外の3状態が消える」は誤り

- **問題**：103行には `Free` も含まれます。
- **証拠**：
  - owner claimは `Free` から取得し、返却時に `Free` へ戻す：`crates/ezsdr-radio-uhd/src/provider/core.rs:628–635,701–712`。
  - F8のreopenもowner claimを使う：`provider/tx.rs:849–864`。
  - report readも `Free` と `reading` を使う：`provider/core.rs:670–696`。
- **訂正**：F5に固有なのは主に `Release`／`FromNone` とその競合処理です。`Free`／`Owner`やreport-read排他の必要性は、F8など残す機能次第です。

### 14. F4(b)(c)・F5(b)：停止状態と全体barrierの条件が不足している

- **問題**：94–95・105行は、RXとTXに同じ「止めたstream」を使っています。
- **証拠**：
  - TX Stopはburstを終え、clockを残し、後のburstは送れる：`design/07-radio-model.md:164–169`、`design/16-easy-api.md:92`。
  - handed samplesは取り戻せない：`design/07-radio-model.md:166,169`。
  - F6はstreamごとのlineと再開を使う：`crates/ezsdr-radio/src/timeline.rs:925–937`。
- **訂正**：TXでは「Stop済み」「hostが今sendしていない」「device上で無音」「後続予約なし」は同義ではありません。全stream停止案でも、F6が残ればstream別の計画は残ります。「line／floor不要」は、全体barrierの予約順・停止完了・再開を定義した後でなければ確定しません。

### 15. F6(b)：start_rxをなくしても「一Run一回だけ流れる」とは限らない

- **問題**：114行の括弧がF4との組合せを無視しています。
- **証拠**：cold changeは新segmentを作ります：`design/07-radio-model.md:235,252–257`、`crates/ezsdr-radio/src/timeline.rs:940–960`。
- **訂正**：なくなるのは明示的Stop後の `start_rx` 再開です。F4を残せば、流れているstreamのcold changeによる停止・再開と複数SampleClockは残ります。

### 16. F7：受付時刻の制限と、preemption／継続機構の廃止は別

- **問題**：121–123行では、予約を受ける時と予約内容の制限が混ざっています。
- **証拠**：
  - 予約済みheld burstも実行時にpreemptする：`crates/ezsdr-radio-uhd/src/provider/tx.rs:498–524`。
  - 隣接継続とheld-tail待ちは別の分岐：同ファイル`:517–537`。
  - 予約済み隣接burstもdevice burstを継続：`design/18-uhd-radio.md:251–253`。
- **訂正**：
  - F7(c)でも、流れ始める前に重なる／隣接するburstを予約できるなら、その処理は残ります。
  - F7(b)の「重なる予約を拒む」は、隙間のある将来burstを許します。「続きの時刻ちょうどだけ受け付ける」は、それより強い別制限です。

### 17. §2：「UHDを使う以上なくならない」と「今回は残す」が混ざっている

- **問題**：29行はhardware必須条件として広すぎます。
- **証拠**：
  - repeat連続性はVisionの要件：`design/vision/10-implementation-path-and-testing.md:213`。
  - timed TX/RX startも同`:215`。
  - re-anchor・reference監視はModuleの方針：`design/18-uhd-radio.md:158–171,345–346`。
  - LateCommand処理はstartを再発行し、通常のstreamer reopenではない：`design/18-uhd-radio.md:228`。
- **訂正**：「今回残す製品機能」「Vision／契約で残すもの」「そのTX動作を安全に続けるためのX3x0制約」を分ける必要があります。EOBやstale-header処理は継続送信時の装置制約ですが、repeatや特定周期の監視そのものがUHDの必須機能ではありません。31行の「開き直し」は「timed startの再発行」が正確です。

### 18. §4：共有状態が「gain releaseとStopだけ」にはならず、三crate同時変更も一律ではない

- **問題**：138–139行は残る共有機構とModule固有規則を省いています。
- **証拠**：
  - loss、delivered floor、clock登録・終了、phase、記録はRX/controlの共有状態：`crates/ezsdr-radio-uhd/src/provider/core.rs:725–870`。
  - 常時残る監視・loss：`design/18-uhd-radio.md:214,352–356`。
  - Mockはnative ownership／flush／report queue寿命を共有しない：`plan/phase8/research/45-concurrency-inventory.md:145–149`。
- **訂正**：F1/F4/F5を制限しても、F6と§2によりloss・終了・進捗・clock・記録の共有は残ります。また、RM上の動作変更はMock/Vocabularyにも及びますが、F3 restoreやF8のnative recoveryの変更が、必ず三crate同じ変更になるわけではありません。

### 19. §3の量・F1の損失：数え方と保証範囲が過大／不十分

- **問題**：
  - 「テスト関数」欄に通常のhelperとnote 45実験の関数が混ざっています。
  - F1の位相保証にはboard・hand-over時刻の条件が抜けています。
  - 再開の代価を50 msだけで把握できる記述になっていません。
- **証拠**：
  - `brain` はtest-onlyの別実験：`crates/ezsdr-radio-uhd/src/lib.rs:8–9`、`src/brain/mod.rs:1–4`。
  - 例えば `restore_rx` と `preempt` はテストではない：`src/provider/rx.rs:435`、`src/provider/tx.rs:1039`。
  - 位相のboard差：`design/18-uhd-radio.md:184`。遅れたhandoverの保証外：同`:285`。
  - successorはcut後にrelease window・旧rateの一receive call・allowance・start leadを要する：`crates/ezsdr-radio/src/timeline.rs:1013–1038`。
- **訂正**：
  - 全Rustファイルの関数名一致は `restore=44`、`from_none=27`、`preempt=10` ですが、実運用側の **`#[test]`付き関数**ではそれぞれ **11、21、9**。`ur_34_` は **49** で、台帳の46ではありません。`ur_24_=137` は再現できますが、F3なども含む共有集合です。
  - timed tuneの位相保証はUBX/OBX等の宣言と期限内handoverに依存し、CBXにはありません。
  - 50 msはstart leadであり、停止から再開までの全所要時間ではありません。

## 機能別の計数

### 方法と限界

**削除可能LOCは分離できません。** 同じ関数・状態が複数機能と§2を実装しています。以下の行数は、指定した**関連production領域の物理行数**です。コメント・空行・領域内のtest hookを含み、機能間で重複します。合計・削減見積りには使えません。

テスト数も排他的な意味分類ではなく、以下の名前検索で再現できる集合です。

- 各crateの `*.rs` を再帰検索。
- `#[test]` に続く関数名だけを採取。helperは除外。
- 同じソース関数は一度だけ数える。feature付き・ignored hardware testも含む。
- UHDの `src/brain/` は別実験として除外。
- 複合的な生成試験や一般名の試験は、名前だけでは各機能へ配賦できません。

使用した名前の正規表現：

```python
patterns = {
 "F1": r"^(?!.*(?:transmit|_tx_))(?:(?:ur_24_|mr_(?:18|34)_|rm_36_).*(?:rx|receive|frequency|ddc|drain|retune|phase)|rx_chain_)",
 "F2": r"^(?:(?:ur_24_|mr_(?:18|34)_).*(?:transmit|_tx_|gain)|ur_22_.*(?:timed.*dsp|t3.*barrier))",
 "F3": r"restore",
 "F4": r"cold|antenna|rate_change|rate_update|refus.*(?:segment|change)|(?:segment|change).*refus",
 "F5": r"^ur_34_|^rm_34_|move|membership|from_none|from_no_stream|empty_stream|join|last_.*channel|channel.*leav",
 "F6": r"stop|start_rx|restart",
 "F7": r"preempt|continuation|held_tail|held_sample|next_burst|back_to_back|burst.*(?:adjacen|target_order)|front.*(?:bound|late)|(?:bound|late).*front",
 "F8": r"recover|reopen|uncertain|failed.*(?:eob|close)|(?:eob|close).*fail|underflow",
}
```

特にF4の初期antenna検証、F5のjoin、F6のProvider stop、F8の基本underflow試験も混ざるため、**この数を「その機能を消せば消えるテスト数」とは読めません**。

### 行数の再現範囲

略号：`U`＝`crates/ezsdr-radio-uhd/src/provider/`、`M`＝`crates/ezsdr-mock-radio/src/lib.rs`、`R`＝`crates/ezsdr-radio/src/timeline.rs`。区間は両端を含み、各区間の `終端−始端+1` を加算しました。

| 機能 | UHDの計数区間（`.rs`） | Mockの計数区間 | Radioの計数区間 |
|---|---|---|---|
| F1 | `core:92–372`; `control:581–952`; `rx:268–488,856–885` | `1024–1188,1974–2037,2190–2223,2261–2321` | `197–199,423–428` |
| F2 | `control:624–952`; `core:1416–1421,1428–1581` | `1033–1188,1974–2037,2190–2223,2261–2321` | `423–428` |
| F3 | `control:380–412`; `core:127–133,353–365`; `rx:435–488` | 該当するnative restoreなし | 同左 |
| F4 | `control:87–219,989–1231`; `core:802–1041,1663–1797`; `rx:502–591`; `tx:1120–1222` | `1330–1453,1514–1636,1974–2187,2190–2260` | `229–375,378–419,853–1039` |
| F5 | `control:964–1231`; `core:628–714,899–958,1663–1731`; `tx:1209–1222` | `2085–2115,2143–2187,2261–2321` | `430–778` |
| F6 | `control:452–510`; `rx:250–341,382–423`; `core:845–870`; `tx:1106–1119` | `1232–1327,1330–1435,1652–1693` | `903–1039` |
| F7 | `control:1233–1345`; `tx:468–764,979–1094` | `299–310,1470–1512,1715–1873,1880–1972` | `795–813` |
| F8 | `tx:180–197,809–964`; `core:1428–1546,1549–1569` | 該当するnative recoveryなし | 同左 |

FakeDevice・native bridgeの共有実装、宣言・初期構成・全体lifecycleなどは上の機能別行数に網羅配賦していません。例えば `crates/ezsdr-radio-uhd/src/device.rs:1407–1519` の `apply` は両方向・timed/untimed共用です。

### 計数結果

各セルは **関連production領域の行数／名前一致したテスト関数数** です。すべて非排他的です。

| 機能 | `ezsdr-radio-uhd` | `ezsdr-mock-radio` | `ezsdr-radio` |
|---|---:|---:|---:|
| F1 | 904 / 73 | 324 / 25 | 9 / 0 |
| F2 | 489 / 13 | 315 / 10 | 6 / 0 |
| F3 | 107 / 11 | 0 / 0 | 0 / 0 |
| F4 | 944 / 57 | 532 / 38 | 376 / 17 |
| F5 | 498 / 125 | 137 / 19 | 349 / 48 |
| F6 | 233 / 136 | 244 / 36 | 137 / 4 |
| F7 | 526 / 32 | 307 / 3 | 19 / 2 |
| F8 | 314 / 47 | 0 / 0 | 0 / 0 |

編集・format・build・test・mutation・実機操作はしていません。行数は共有領域の計測であり、削減量の保証ではありません。

**FACTS NEED CHANGES — 19 findings**
