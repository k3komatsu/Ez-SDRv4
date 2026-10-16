現案では、設定の置き場より先に詰めるべき点が見つかっています。特に、channel の移動を2本の stream へ整合的に反映する規則と、XG の合計帯域判定が実際の link 割当てでも成立するかを確認しています。受理済みの P1〜P5 は前提のまま扱います。
## 結論

**計画・設計33とも、変更つきで進めてよい。** 受理済みの P1〜P5 と channel／stream 分離は維持するが、D9 の帯域判定は実機より緩くなり、membership の切替・selector の表現・複数 device の tree も Gate P 前に確定する必要がある。

ファイル変更・build・test・実機操作は行っていない。以下、**VERIFIED** は直接確認した仕様・コード・UHD v4.10.0.0 の事実、**INFERRED** はそこからの設計判断を示す。

## 指摘

### 1. P0 — D9 の合計帯域＋単一 channel 上限では、XG の実機限界を守れない

**対象：設計33 §4 D9**

**VERIFIED：** UHD の自動 link 選択は、帯域ではなく RX／TX 接続の**割当て数**を比較する。異なる rate の channel を帯域均等に振り分ける保証はない。

**INFERRED：** 例えば channel 接続順が 200、25、200、25 Msps なら、交互割当てで片方が 400 Msps＝1.6 GB/s になる。D9 の条件は、合計1.8 GB/s ≤ 2 GB/s、各 channel ≤ 1 GB/s なので通すが、片方の10 GbEだけで過負荷になる。

**修正：** 共有の判定に、実際の割当てを再現できる規則か、割当てを保証できない場合の保守的な制限を入れる。再 open を含む割当て履歴も対象にする。「これ以上約束しない」では §59 を満たさない。XG 対応という owner 決定を撤回する必要はない。

根拠：
- `plan/phase8/33-channels-and-streams.md:128`
- [UHD graph_stream_manager.cpp:188–219,252–284](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/graph_stream_manager.cpp#L188-L284)
- `design/vision/10-implementation-path-and-testing.md:105`

### 2. P1 — membership の `cold` だけでは、2本の stream を安全に切り替えられない

**対象：設計33 §4 D3、§5**

**VERIFIED：** 現行 timeline は stream ごとの `channels` と `ratio` を持ち、cold の effective instant の floor も stream ごとである。channel の集合・移動先・移動元は保持しない。

**INFERRED：** c1 を s0→s1 とするとき、次が未定義である。

- 両 stream に先行 cold がある場合の共通 effective instant。
- 古い streamer が物理 channel を解放する前に、新しい streamer が取得しない規則。
- 一方の設定／read-back だけ失敗した場合の halt・拒否・再計画。
- channel 数が同じまま構成員だけ変わる場合の再 open と新 SampleClock。
- 旧 TX waveform の channel 対応と held burst の取消し。

**修正：** membership 更新を Provider 内の**一つの移動操作**として計画し、影響する両 stream の booking・cut・解放・開始・失敗処理を定義する。Kernel の汎用 transaction は不要。channel mapping は segment ごとに固定・記録する。

根拠：
- `plan/phase8/33-channels-and-streams.md:76`
- `crates/ezsdr-radio/src/timeline.rs:10–17,195–220,320–335`
- `design/07-radio-model.md:175–177,251–255`
- `design/18-uhd-radio.md:208–210`

### 3. P1 — D5 の selector 例は現在の Kernel 文書として書けない

**対象：設計33 §4 D5**

**VERIFIED：** `Binding.selector` は `Map<Ident, Value>`、`Value` の list／map の中身は scalar に限られる。`devices: [{ master_clock_hz, rx: [...], … }]` は list 内の object と、その中の list を含むため拒否される。

**修正：** Kernel を広げず、device ごとの scalar map／channel の文字列 list など、既存 `Value` で書ける平らな形式にする。Gate P に実際に parse できる BindingProfile 例を置く。

根拠：
- `plan/phase8/33-channels-and-streams.md:90–95`
- `design/03-spec-and-binding.md:94,150–151`

### 4. P1 — 「timekeeper＝device」と D10 の tree が一致していない

**対象：設計33 §3、§4 D10**

**VERIFIED：** owner 決定は「timekeeper 1つを device node 1つ」。一方、設計33は通し番号の全 channel／stream を `<dev>` 一つの下に置き、device の違いを layout にだけ持たせている。

**INFERRED：** この `<dev>` が instance 全体なら device の定義と異なり、timekeeper 単位なら別 device の channel／cross-device stream の所属が説明できない。さらに、複数の `radio.device` を作るだけでは、Session の「root kind から resource 一つ」と D1 の全設定一括検査が自動的には両立しない。

**修正：** instance の container、timekeeper 単位の device、instance が所有する stream の関係を図で固定する。root 間の時刻関係の決定は8bへ延期してよいが、**所有 tree と番号の scope**は今決める。

根拠：
- `plan/phase8/00-overview.md:144`
- `plan/phase8/33-channels-and-streams.md:30–46,134`
- `design/03-spec-and-binding.md:233–243,294–299`

### 5. P1 — D1 は機能的には成立するが、受理済み (c) と同じ説明にはなっていない

**対象：設計33 §4 D1**

**VERIFIED：** 受理された (c) は channel node に設定と capability を持たせる説明。D1 はそれらを device node に集約し、channel node を空にする。また、退けた K2 の `fragment × node path × key` は、必ずしも node ごとに fragment を作る案ではない。

**INFERRED／推奨：** **D1 を採る方がよい。** 理由は変更量ではなく、一つの所有者の設定全体を既存の純粋な `coerce` と admission に渡せ、共有制限を一貫して判定できるため。ただし、K2 を「必ず fragment をまたぐから」と退ける理由は訂正する。

番号は**実験の論理 channel 番号**、layout はそれを物理 frontend に対応付ける binding と定義すれば、Spec／binding 分離に反しない。capability が同じ2本のケーブルの入替えは matcher では検出できないので、「capability が誤配線も拒否する」とは約束しない。owner には配置の変更を明示して確認する。

根拠：
- `plan/phase8/00-overview.md:82,137`
- `plan/phase8/33-channels-and-streams.md:52–64`
- `design/03-spec-and-binding.md:160–163`
- `design/06-kernel-coordinator.md:333–337`

### 6. P1 — D2 の成立条件は「lookup を足す」より広い

**対象：設計33 §4 D2、§6**

**VERIFIED：** SB-2 の5位置は共通 iterator と `registry.key_decl` を使い、SB-45 の declaration default も同じ lookup に到達する。したがって、番号付き宣言を中央で解決する方向はよい。

ただし以下を決める必要がある。

- segment index の数え方・範囲・Vocabulary prefix 内の指定禁止。
- ASCII 数字、`c0` と `c01` の別名問題、長大な数字。
- exact 宣言と indexed 宣言の重なり、複数 indexed 宣言の衝突。
- 非具体的な `radio.rx.c.frequency_hz` を実際の要求 key として受け付けるか。
- `policies.coercion` は具体 key の完全一致のままか。
- 存在しない c999 を Provider が必ず拒否すること。

最後の点は重要で、**coercible key は capability がなくても `coerce` に進む**。capability の不存在だけが上限を守る、という説明は不十分である。

**修正：** policy は具体 key の完全一致を維持し、pattern policy は導入しない。5位置・validate／plan／prepare／Session admission を通すテストと、`KeyDecl` を含む派生 schema の更新を段階1b-1に明記する。

根拠：
- `crates/ezsdr-kernel/src/spec.rs:508–525,1171–1199`
- `crates/ezsdr-kernel/src/module_api/registry.rs:194–198`
- `crates/ezsdr-kernel/src/plan/validation.rs:504–557`
- `crates/ezsdr-kernel/src/plan/matching.rs:102–135`
- `crates/ezsdr-kernel/src/plan/coercion.rs:5–17`

### 7. P1 — D8／D9 は、runtime まで「coerce の中だけ」では完結しない

**対象：設計33 §4 D8・D9**

**VERIFIED：** KC-26 が渡すのは、時刻ごとの適用状態ではなく KC-27 の**admitted configuration**。`Requested` に deadline や queue 状態はない。現行 Mock／UHD も booking 時に projected configuration を検査し、queue は別に管理している。

**INFERRED：** 共有資源の静的な組合せ判定は共通 `coerce` に置ける。しかし、未来の membership 更新、逆順時刻の操作、read-back 拒否後の状態、queue の空き・release 順序は別の runtime 規則が必要。

queue は radio／DDC／DUC のどれを一操作が消費するか、共有範囲、保持中と発行済みの数え方を定義すること。「slot ごと」に分けるだけでは DSP tune を含む操作の footprint が決まらない。

根拠：
- `design/06-kernel-coordinator.md:333–337`
- `crates/ezsdr-mock-radio/src/lib.rs:1318–1348`
- `crates/ezsdr-radio-uhd/src/provider/control.rs:391–427,434–479,502–528`
- `plan/phase8/33-channels-and-streams.md:123–129`

### 8. P1 — 共通 description に、実際の共有制約と推測の制約が混在している

**対象：設計33 §4 D5・D8**

**VERIFIED：**

- X300 の RX aggregate gain group は、各 frontend に共通の ADC gain を含む。ADC gain の設定は A/B 両方に作用する。
- D5 は daughterboard clock の競合を「UHD が開くときに拒否」としているが、Mock 側の同等拒否が定義されていない。
- Basic/LF の antenna 設定は frontend ごとの接続に作用する。今回読んだ実装では「AB/BA を使うと他 frontend を使えない」という検査は確認できなかった。

**修正：** Basic/LF の「gain なし」は **board の RF gain がない**意味なのか、UHD aggregate gain を使わない意味なのかを固定する。共有 ADC gain・clock 競合を共通規則で扱う。AB/BA の排他は未検証のまま規範化しない。

根拠：
- `plan/phase8/33-channels-and-streams.md:101–106,123`
- [x300_radio_control.cpp:1303–1309,1718–1739](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L1718-L1739)
- [x300_adc_ctrl.cpp:78–86](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_adc_ctrl.cpp#L78-L86)
- [x300_radio_control.cpp:296–312](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L296-L312)

否定検索：UHD の `db_basic_and_lf.cpp` に対する `other.*chan|exclusive|conflict|already.*use|in.use`、`x300_radio_control.cpp` に対する `basic.*(conflict|exclusive)|(?:conflict|exclusive).*basic` は、どちらも大文字小文字無視で0件。これは実機で許されることの証明ではない。

### 9. P1 — aligned stream と known-phase coherent stream を区別する必要がある

**対象：設計33 §3、§4 D5・D12、計画の CBX 範囲**

**VERIFIED：** Vision §25 は CoherentGroup に sample alignment と既知の位相関係の両方を要求する。現行 CBX description の `random_unless_timed_tune` は、コード自身が過大宣言と記録している。計画は CBX を追加対象にしながら、範囲外欄では「機材に CBX がない」として `random` を除外している。

**修正：** 異設定 channel の aligned block は認める。一方、同 rate・同 timestamp だけで `coherent=true` にしない。stream の coherence は構成員・位相挙動から判定し、CBX に known-phase 保証を出さない。`random` を延期するなら、保証しない範囲を明記する。

根拠：
- `design/vision/05-coherence-calibration-and-runtime-semantics.md:14–37`
- `crates/ezsdr-radio-uhd/src/profile.rs:76–78,102–105`
- `plan/phase8/00-overview.md:40,74,82`
- `plan/phase8/33-channels-and-streams.md:102,144`

### 10. P1 — D7 は現行の受信 antenna 安全検査を落としている

**対象：設計33 §4 D7**

**VERIFIED：** RM-19 は送信条件に加え、受信 antenna も `antenna_ports` と照合する。D7 は送信 channel の検査だけを列挙している。

**修正：** 受信 antenna の検査も channel ごとに残す。`tx_enabled[n]` は stream 内の位置ではなく固定の論理 TX channel 番号とし、membership 変更で対応を詰め直さない。

根拠：
- `design/07-radio-model.md:188–203`
- `plan/phase8/33-channels-and-streams.md:119`

### 11. P1 — D6 の bool capability は将来の timed-update 制約を解決したことにはならない

**対象：設計33 §4 D6**

**VERIFIED：** `hardware_timed` は明示的な `at` がない更新にも適用される。`timed_tune=false` を Provider 到着後に拒否するだけでは、非対応装置上の通常の frequency setter もそのままでは動かない。また調査対象の X420 は周波数帯により timed tuning の可否が変わる。

**修正：** 今回は X3xx だけ実装するので Kernel の動的 update-class 機構を追加しなくてよい。ただし、bool は能力の情報であって admission の解決ではないと明記し、非対応装置の実装前に、値依存の可否と untimed 更新を設計する。

「省略可能な `ParamDecl.update_class` と一緒に決める」は、その optional 化が既に実装済みなので記録を直す。

根拠：
- `design/05-module-api.md:236`
- `design/04-run-and-session.md:261`
- `plan/phase8/33-channels-and-streams.md:112–115`
- `plan/phase8/research/usrp-family-limits.md:119`（X420 の性質は本レビューでは一次資料の再照合未実施）

### 12. P1 — 段階分割が、現状のままでは「各段階で緑」を保証しない

**対象：計画 §5、設計33 §5、D11**

**VERIFIED：** EA-9 の default profile は旧 profile 名・port `rx` を使う。Python も旧 key と `/rx`・`/tx` を使い、capture は feed の一致で recorder を選ぶ。これらの改訂が、profile 改名や旧 key 削除より後の段階に置かれている。

**修正：**

- 1a の profile 改名時に default profile と全利用箇所を同時更新する。
- 1b は内部の追加・移行を先にし、旧 key／port の削除は Mock・UHD・server・Python が揃う切替段階にまとめる。
- EA-17 も改訂対象に加え、`rx_streams[k].capture` が `rxk` の recorder を選ぶ規則を定義する。
- `radio.rx.frequency` の全 channel 更新は複数 Action になる。途中拒否時に一部だけ変わり得ること、異設定時の getter をどう扱うかを決める。互換性のためだけの糖衣は必須ではない。

また、1a の rate grid 修正は「値は段階4まで変えない」の明示的な例外にする。既知の誤 grid を測定待ちで残す必要はない。

根拠：
- `plan/phase8/00-overview.md:97–102`
- `plan/phase8/33-channels-and-streams.md:140,150–157`
- `design/16-easy-api.md:45–63,83–90,106`

### 13. P1 — P2／P3 は維持できるが、測定の適用範囲と bench 表を揃える必要がある

**対象：計画 §4 P2・P3・P5、§6・§7、設計33 §6**

**VERIFIED：** P5 は2台目に OBX を移さず測る決定だが、BP3 と出口条件は2台とも OBX。XG は受理済みだが bench 表に明示的な XG 行がない。

**INFERRED：** `x3xx` という profile 名だけで「全記録の最悪値」を比較すると、1／2 link、board、rate、同時 stream の異なる測定が混ざる。restart readiness の receive-call 項も rate／packet 条件に依存する。

**修正：** 記録にある構成情報を、比較する値の**適用条件**として使う。数値以外の rate 集合・antenna 集合・共有拘束にも、包含／拒否関係を定義する。XG、混在同時運転、再 open 後、RX/TX 同時負荷を bench matrix に明示する。未測定を許す P3 は変えず、未測定領域まで実測 parity 済みとは呼ばない。

根拠：
- `plan/phase8/00-overview.md:83–86,113–126,142`
- `crates/ezsdr-radio/src/timeline.rs:376–395`
- `design/09-mock-radio.md:65–66`

## UHD C API の確認結果

以下はすべて **VERIFIED、tag v4.10.0.0**。

| 項目 | 確認結果 |
|---|---|
| rate の候補取得 | `uhd_usrp_get_rx_rates`／`uhd_usrp_get_tx_rates` がある |
| subdev spec 設定 | `uhd_usrp_set_rx_subdev_spec`／`uhd_usrp_set_tx_subdev_spec` がある |
| 複数 streamer | streamer handle を複数作り、それぞれに `uhd_usrp_get_{rx,tx}_stream` を呼べる。C wrapper は指定された handle に streamer を格納する |
| 現行 v4 の制約 | UHD C API の不足ではなく、v4 の `Device` と `UhdDevice` が各方向1 streamer だけ持つ設計 |

根拠：
- [usrp.h:116–193,288–294,494–521,682–709](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/include/uhd/usrp/usrp.h#L494-L521)
- [usrp_c.cpp:256–287](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/usrp_c.cpp#L256-L287)
- [multi_usrp_rfnoc.cpp:325–407](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L325-L407)
- `crates/ezsdr-radio-uhd/src/device.rs:163–190`
- `crates/ezsdr-radio-uhd/src/uhd.rs:741–758`

D5 の照合は、単なる min／max ではなく rate 集合を比べること。DDC の入力 rate が未確定の場合、`get_output_rates` は `{1.0}` を返す経路もあるため、照合に必要な初期化順を定義する。

根拠：[ddc_block_control.cpp:153–166](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/ddc_block_control.cpp#L153-L166)。

## 設計33 §7 の5つの問いへの答え

1. **D1／D2 は (c) の意図を満たすか。K2 を退けてよいか。**  
   **機能的には満たせる。D1 を推奨する。** ただし「channel node 自体が設定・capability を持つ」という受理文とは異なるため、明示的に確認する。K2 を退ける理由は変更量ではなく、所有者全体の検査を既存モデルで完結させること。K2 が必ず複数 fragment になるという説明は撤回する。

2. **D2 は5位置・policy・schema に穴を作らないか。**  
   **中央 lookup への統合で防げるが、現案だけでは未確定。** 指摘6の名前・重複・具体 key・上限拒否を定義する。`policies.coercion` の wildcard 化は不要。既存の component `ParamDecl` 例外を変えない。

3. **membership を cold にしたときの区切りは。**  
   **移動元・移動先の両 stream を区切り、新しい構成員集合で新 SampleClock を作る。** 同じ channel 数・rate でも再生成する。各旧 clock の cut は RM-16、新 origin は RM-25に従い、間は gap にしない。物理 channel の二重使用を防ぐ handover と失敗処理を共通に定義する。

4. **D8／D9 を coerce だけで判定できるか。**  
   **静的な組合せはできる。runtime 全体はできない。** KC-26 の検査に加え、適用時刻へ投影した設定の再検査、queue accounting、release 順序、read-back 拒否後の処理が必要。共通実装を語彙に置き、両 Provider が使う。

5. **8b／X420／X440 を締め出す決め方は残るか。**  
   **残る。** device／instance の tree の混同と、D9 の均一な link 合算は今直す。複数 root の時刻関係、dual-rate、値依存 timed tuning の実装は後でよいが、「有理数比なら coherent」とはしない。通し番号自体は、論理番号として扱えば障害ではない。

## D1〜D12 への賛否

| 判断 | 賛否 |
|---|---|
| **D1** | **条件つき賛成**：device／instance 単位の平らな設定を推す。論理 channel の意味と owner 決定との差を明示する |
| **D2** | **条件つき賛成**：一般的な番号付き宣言に限定し、曖昧性・policy・全入口の検査を固める |
| **D3** | **条件つき賛成**：scalar membership と cold はよい。複数 stream の handover 規則が必要 |
| **D4** | **賛成**：固定 node／port は構造不変に合う。「未使用でも node は存在、clock は不存在」と整理する |
| **D5** | **条件つき賛成**：共通 description は正しい。selector の文法、共有 ADC gain、clock 競合、rate 集合照合を直す |
| **D6** | **限定的に賛成**：X3xx の今回実装には十分。将来装置の admission／untimed 更新を解決済みとはしない |
| **D7** | **条件つき賛成**：channel 単位化は必要。受信 antenna 検査を維持する |
| **D8** | **条件つき賛成**：共有検査は必要。Basic/LF の推測の排他を規範にせず、実在する共有制約を入れる |
| **D9** | **現案に反対**：帯域式が不十分。queue の操作 footprint と runtime 規則も必要 |
| **D10** | **現案の tree 記述に反対**：長さ1だけの実装には賛成。timekeeper＝device の決定に所有構造を合わせる |
| **D11** | **条件つき賛成**：明示的 channel／stream API を主にし、capture feed・getter・部分更新を決める |
| **D12** | **賛成**：旧 key／profile 名を削除してよい。全利用箇所と同時切替し、version は上げない |
