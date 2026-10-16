読み取り専用で、Spec の matching・Session の宛先まで追います。UHD は指定の v4.10.0.0 を確認し、ソースから確認できる事実と、実機測定が必要な性質を分けて報告します。
提案 (b) には、設計メモで解く必要のある穴が見つかりました。特に BasicRX の A/B 独立受信は、同じ `antenna` を全 channel に適用する現行規則では表せません。また、binding ごとに selector を変えてグループを選ぶと、SB-3 上は別 Provider instance になるため、「1台を1 instance が所有する」という狙いと衝突します。
## 結論

**(b) を変更つきで採る。** ただし node は「board の種類」ではなく、**1本の stream と同一設定を共有する論理 channel 集合**とし、BasicRX の A/B、binding の instance identity、共有 envelope を先に設計する必要がある。

現状の提案のままでは、例示された **BasicRX の A/B 独立受信を表せず**、「v3 の `for-channels` と1対1」という説明も正しくない。

以下、**VERIFIED** は仕様・実装・指定 tag のソースで確認した事実、**INFERRED** はそこからの設計上の判断を示す。ファイル変更・build・test・実機操作は行っていない。

## 指摘

### 1. P1 — 穴は実在する。ただし「混在なら何も表せない」は言いすぎ

**VERIFIED**

- RM-4 は方向ごとに frequency・gain・antenna を1値だけ持ち、RM-5 は **stream の全 channel がその値を共有する**と明記している。
- したがって UBX RX を 1 GHz・`RX2`、BasicRX を 0 Hz・`A` に設定するような要求は表せない。
- 一方、異種 board でも全 channel が共通設定を受け入れる場合まで、RM-4 の形が数学的に禁止するわけではない。**一般の混在構成を表現できない**ことと、**混在そのものを禁止する**ことは別。
- 現実の v4 は UR-5 の frontend 照合と profile の固定値でも対象を狭めている。

根拠：`design/07-radio-model.md:63–72,97–99`、`design/18-uhd-radio.md:116–118`、`crates/ezsdr-radio-uhd/src/profile.rs:58–85`。

さらに、**同種 UBX 2枚を別周波数にするだけでも同じ穴がある**。問題の本体は board の混在ではなく、設定対象と stream 全体を同一視していること。

### 2. P0 — BasicRX の「A/B なら2ch」は、(b) の現行設定規則と両立しない

**VERIFIED**

UHD 4.10 は Basic/LF RX に frontend `"0"` と `"1"` を公開し、各 frontend の antenna として `AB/BA/A/B` を扱う。`A→I`、`B→Q` である。antenna の変更は指定 channel の frontend core に作用する。

根拠：[U1:26–33](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.hpp#L26-L33)、[U2:296–312](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L296-L312)。

**INFERRED**

独立な A/B 受信には、一方の frontend に `A`、他方に `B` を設定する必要がある。同じ node の2chへ同じ `radio.rx.antenna` を適用しても、それにはならない。

設計メモでは次のどちらかを必ず選ぶこと：

- scalar 設定を維持し、BasicRX の独立 frontend を別グループにする。
- 2ch を1 stream に保つため、**同一 stream 内の channel 別設定**を導入する。

前者を推奨するが、後述の coherence 制約を明示する必要がある。「board 1枚＝node 1つ」の固定対応は採らない。

### 3. P0 — 設定グループと coherent stream の境界は同じではない

**VERIFIED**

Vision §25 は、aligned multi-channel samples を **1 Provider の1 stream**から出すと定める。独立 stream の集合は、時計配線が同じでも DeviceGroup である。現行 `radio.rx.coherent` の保証も **その node の1 stream 内**に限られる。

根拠：`design/vision/05-coherence-calibration-and-runtime-semantics.md:20–37`、`design/07-radio-model.md:112–113`。

**INFERRED**

同一 root と timestamp は、異なる stream のサンプル時刻を比較する手段であって、欠落・再開・位相関係まで一体に保証する aligned block の代替ではない。

(b) では：

- 同設定の UBX 2枚など、既存の coherent multi-channel 受信は1グループで残す。
- 異設定のグループを分けた場合、グループ間 coherence を暗黙に保証しない。
- 「異なる gain/antenna のまま1本の coherent stream が必要」という要求は、(b) の scalar 設定だけでは満たせないと記録する。

この制限を owner が受け入れないなら、推奨を **channel 設定 node と stream node を分ける案**へ変えるべきである。

### 4. P1 — selector に slot 構成を書くだけでは、各 resource の宛先は決まらない

**VERIFIED**

SB-3 は `(module, selector, profile)` 全体を instance identity とする。同じ instance の複数 sub-resource を使うなら、binding description は同一でなければならない。

一方、matcher は node の `kind` と capability 制約で選ぶ。Spec resource 名を node 名として照合したり、selector 内の group 指定を node 選択に使ったりはしない。実装は greedy である。

根拠：`design/03-spec-and-binding.md:148–149,294–299`、`crates/ezsdr-kernel/src/plan/matching.rs:25–54`。

**INFERRED**

binding ごとに `selector.group = ...` を変える方法は、現在の規則では別 instance を作る。同じ X300 を1 instance が所有する案の解決にはならない。

設計メモには、少なくとも次を示す必要がある：

- 同じ Provider tree の複数 group に、各 Spec resource がどう一意に matching するか。
- 同能力の group が2つあるとき、物理配線との対応をどう固定するか。
- coercion を要する候補の選択や resource 名順で、意図しない group が選ばれないか。

slot 名を Spec に直接書いて解決すると intent/binding 分離を弱める。**node 選択だけのために instance identity や Kernel の matching を大改造しないこと。**

### 5. P1 — Session は自動では group を選べない

**VERIFIED**

Session の implicit Spec は binding ごとに **Provider root の kind**から resource を作る。Action は matched resource 以下の path に解決される。radio の update は device node、送受信操作はその `/rx`・`/tx` を宛先とする。

根拠：`design/03-spec-and-binding.md:185,208–209,233–243`、`design/07-radio-model.md:153–155`、`design/09-mock-radio.md:140,175`。

**INFERRED**

container root の下へ group node を置くだけでは、Session がどの group を操作するか未定義になる。最低限、以下を設計すること：

- `set`・`send`・`start_rx`・`Stop` の group 指定。
- group Stop と device 全体の Stop の区別。
- 異なる rate の group がある場合の schedule clock。
- Session capture の feed と、group ごとの effective configuration。

Spec Run の例だけでなく、**Session の一連の操作例**が必要である。

### 6. P1 — `DeviceDescription` は、そのままでは片方向 board に再利用できない

**VERIFIED**

現行 description は frequency・gain・max_channels を RX/TX 共通で1つ持つ。RM-26 も同じ形である。

根拠：`crates/ezsdr-radio/src/device.rs:164–205`、`design/07-radio-model.md:263–277`。

**INFERRED**

`rx.channels = 0` を default にするだけでは、TX-only board の RX capability を0に制限できない。少なくとも方向別の最大 channel 数が必要である。slot node に LFTX＋BasicRX をまとめるなら、方向別 frequency・gain 範囲も必要になる。

また Basic/LF は RF frontend の周波数値が0固定で、通常の tune では DSP 側が周波数差を受け持つ。**frontend の ±max_freq を、そのまま最終的な `radio.*.frequency_hz` の許容範囲にコピーしてはいけない**。UHD の総合 tune range は DSP range と bandwidth も参照する。

根拠：[U3:124–149](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L124-L149)、[U4:1373–1404](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L1373-L1404)。

### 7. P1 — group ごとの envelope を並べると、共有資源を過大宣言しやすい

**VERIFIED**

RM-7 の帯域判定は device description 内の `channels × rate × wire_bytes_per_sample`。`coerce` は bound node ごとに呼ばれ、純粋関数でなければならない。

根拠：`design/07-radio-model.md:105–108`、`design/03-spec-and-binding.md:160–161`、`design/05-module-api.md:163`。

**INFERRED**

各 group に motherboard の 1 GB/s を丸ごと付けると、個別には通っても合計が物理リンクを超える。同様に、command queue、配送 allowance、control thread 負荷、loss・reference 状態には共有範囲がある。

必要なのは、次の区別である：

- group 固有の制限。
- motherboard／transport／Provider 全体の制限。
- 複数 group の組合せで初めて生じる制限。

**「値を1か所へ置いた」だけでは parity は成立しない。** 静的な保守的割当てか、全 fragment を対象とする Vocabulary 側の検査など、共有予算の拒否方法を決める必要がある。純粋な `coerce` に他 fragment の処理順依存状態を持たせないこと。

### 8. P1 — UHD 境界の変更は profile 整理より大きい

**VERIFIED**

現行 `Device` は `rx_open(channels: usize)` など、各方向1 streamer を前提とする。実装は channel `0..channels` を使い、open 時に以前の RX streamer を置き換える。

根拠：`design/18-uhd-radio.md:76–86`、`crates/ezsdr-radio-uhd/src/uhd.rs:741–758`。

**INFERRED**

(b) の実装には、group ごとの物理 channel リスト・streamer identity・状態管理が必要となる。変更対象は少なくとも：

- `Device` trait、UHD 実装、FakeDevice。
- Provider の複数 fragment prepare と group 別実行状態。
- MockRadio の MR-7、fault・SimulationChannel の channel mapping。
- clocks、events、bursts、Manifest sections の group 帰属。

Kernel は既に **prepare は fragment ごと、arm/start/stop は instance ごと**を規定している。この仕組みを使い、group ごとに別 Module を作らない。

根拠：`design/05-module-api.md:149–150`、`design/09-mock-radio.md:91–101,167–183`。

### 9. P1 — 8b・8c へは「同じ所有者、別の資源境界」を維持する

**VERIFIED**

SB-23 の複数 motherboard は1 Provider instance。MA-3 は concrete Module 間依存を禁止する。

根拠：`design/03-spec-and-binding.md:266–267`、`design/05-module-api.md:137`、`design/vision/11-boundaries-invariants-and-roadmap.md:337`。

**INFERRED**

推奨構造は、**1 Provider root → motherboard・frontend・stream group の対応**を Provider 内で管理する形。group を「slot 1個」に固定すると、8b の複数 motherboard をまたぐ coherent stream に合わない。

8c の DDC/DUC・Replay も、同じ所有 Provider が宣言する別 capability/resource として扱える。ただし Phase 8 で RFNoC graph の汎用モデルや自動配置まで作るのは scope creep である。

### 10. P2 — v3 の表現力は実装で確認できるが、(b) と1対1ではない

**VERIFIED**

v3 は `for-channels` に指定した各 channel に rate・freq・gain・ant を設定し、RX/TX 別の subdev spec を設定していた。

根拠：`v3/cpp/uhd_usrp/multiusrp.cpp:90–108,126–147,178–184,199–217,349–379`。

さらに、streamer の channel 集合は `tx-streamers[].channels` から**別に**選んでいる。

根拠：`v3/cpp/uhd_usrp/multiusrp.cpp:387–412`。

したがって：

- 「v3 は channel 集合ごとの設定を表せた」――正しい。
- 「その設定集合が必ず1 streamer だった」――正しくない。
- 古い BasicTXRX fixture は TX/RX 別 subdev の証拠だが、UBX＋LFTX＋BasicRX の実行確認の証拠ではない。

根拠：`v3/config_examples/old/x310_BasicTXRX.json:3–20`。v3 の構造を移植する理由ではなく、必要な振る舞いの証拠として使うべきである。

## UHD 4.10 の確認結果

以下はすべて **tag `v4.10.0.0`**。

| 問い | 結果と確度 | 根拠 |
|---|---|---|
| 各 slot に別の RX board／TX board を使えるか | **VERIFIED：driver は対応する。** slot ごとに RX/TX EEPROM を別々に読み、別 ID で daughterboard manager を作る。manager は standalone RX/TX を独立に初期化する。UBX の RX/TX 半分を任意に別物へ置換できる、という意味ではない。 | [X300:1503–1529](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L1503-L1529)、[1575–1584](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L1575-L1584)、[manager:395–474](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard_manager.cpp#L395-L474) |
| X300 で Basic/LF mode は antenna で選ぶか | **VERIFIED：選ぶ。** X300 は Basic/LF ID を RFNoC 用へ変換する。`AB` は初期値であって固定値ではない。`AB/BA/A/B` の変更が指定 channel の mux／frontend 接続に反映される。 | [X300:1515–1528](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L1515-L1528)、[264–312](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L264-L312)、[Basic/LF:95–98,150–168](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L95-L168) |
| RX streamer を複数作り、異なる rate にできるか | **VERIFIED：独立した channel/DDC chain に対する複数 streamer と channel 別 rate 設定を実装している。** 既存 streamer を壊さず追加する処理があり、DDC output rate は channel ごと。一方、**1 streamer 内は rate を揃える処理がある**。 | [multi_usrp_rfnoc:325–407](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L325-L407)、[1314–1332](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L1314-L1332) |
| X300 標準 image に独立した RX chain があるか | **VERIFIED：HG image は共通構成を継承し、slot ごとに2-port DDC と2本の RX 接続を持つ。** 任意の FPGA image でも同じとは限らない。 | [HG:13–14](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/top/x300/x300_HG_rfnoc_image_core.yml#L13-L14)、[共通構成:38–59,78–95](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/top/x300/yaml_include/x3xx_radio_base.yml#L38-L95) |
| channel と subdev の対応 | **VERIFIED：RX/TX 別に、motherboard 順、その motherboard の subdev spec 順で global channel が形成される。** 各 pair は slot 名と frontend 名で Radio block/channel に対応する。したがって「global channel 1＝slot B」は一般には誤り。 | [初期 mapping:205–278](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L205-L278)、[RX:1212–1261](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L1212-L1261)、[更新時:2803–2826](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L2803-L2826) |
| Basic/LF の帯域定数 | **VERIFIED：Basic は `250e6`、LF は `32e6` Hz。** 公開 bandwidth は A/B で定数×1、AB/BA で×2。つまり Basic は250/500 MHz、LF は32/64 MHzという driver 値になる。**X300 の持続 sample rate や実測 RF 通過帯域ではない。** | [定数・係数:22–33](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.hpp#L22-L33)、[RX bandwidth:143–163](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L143-L163)、[TX:240–260](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L240-L260) |

**INFERRED／未測定：** 指定構成での同時持続 throughput、異 rate の同時運転時の lead・restart、各実機の位相関係・アナログ帯域は、このソース確認では保証できない。また UHD 自身が daughterboard clock の要求衝突による組合せ制限を記載しているため、種類の直積を無条件に許可してはいけない。

根拠：[X3x0 docs:535–548](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/devices/usrp_x3x0.dox#L535-L548)。

## (b) を採る設計メモで決めること

1. **group の定義**：board／slot ではなく、どの設定・rate・stream を共有する集合か。BasicRX A/B の具体例を必須にする。
2. **tree と matching**：root kind、group kind、同能力 group の選別、同一 binding identity。
3. **物理 mapping**：motherboard・slot・方向・frontend・UHD channel・stream channel の順序と照合方法。
4. **所有と排他**：group 間で同じ物理 channel を重複使用しない。異なる node ID なら SB-34 は物理重複を検出しない。
5. **Session／Action**：group 指定、全体 Stop、clock 選択、capture feed。
6. **description**：方向別 channel 上限・範囲・default、Basic/LF の tune・antenna・gain の意味。
7. **共有制限**：transport、queue、配送負荷、reference・device loss の影響範囲と拒否段階。
8. **時刻と coherence**：group ごとの SampleClock、rate 変更時の新 clock、同時開始と aligned stream の保証の違い。
9. **parity 記録**：profile 名 `x3xx` だけでなく、slot 構成・mode・group/channel mapping・rate・同時負荷・UHD/FPGA を記録する。OBX 単独測定を混在同時運転の証明にしない。
10. **変更範囲と gate**：RM-2/3/5/9/26、MR-7、UR-2/3/5/9、Session と schema・fixture・mutation rows。現行 coherent multi-channel 動作を回帰させない。

P1 の共通 description 化と、混在・複数 streamer 対応は**別の実装段階**に分けるべきである。ただし最初の段階で、後者を不可能にする board 固定の tree を確定しない。

## 推奨する案と、退ける案

| 案 | 判断と理由 |
|---|---|
| **変更版 (b)：1 Provider 所有、論理 stream group ごとの node** | **推奨。** scalar matching・coerce を再利用でき、独立 rate を自然に持てる。ただし BasicRX frontend 分割と cross-group coherence の非保証を明記する。 |
| (a)：channel ごとの動的キー | 退ける。現行の厳密な KeyDecl 登録とも合わず、channel 数だけキーが増える。ただし「1 stream 内で frequency/gain/antenna を変える」能力自体は有用で、rate の議論だけで否定できない。 |
| **channel 設定 node と stream node を分離** | 今回の第一案にはしない。同 rate・異設定の coherent stream には最も素直だが、channel と stream の割当て・Action・排他まで設計する必要がある。(b) の制限を受け入れられない場合の代案。 |
| 1台を複数 Provider instance で共有 | 退ける。共有 handle・時刻初期化・queue・cleanup の所有問題を増やし、SB-23／8b の方向にも逆行する。 |
| slot ごとの node | 退ける。LFTX＋BasicRX の方向差は表せても BasicRX 内の独立設定を解かず、slot をまたぐ coherent group を不自然に分断する。 |
| group を binding だけに隠す | **物理 mapping だけなら採る。** 独立な設定・stream・Action 宛先まで隠すのは退ける。それらは実験の intent／graph に現れる必要がある。 |
| list/map 値による channel 設定 | 今回は退ける。Session の値には使えても、現行 SB-5 の constraint/capability は scalar のみであり、Spec matching の穴が残る。 |

**要点は「組合せ profile を増やさない」ことではなく、物理 board・設定対象・stream・Provider 所有者を混同しないことである。**
