読み取り専用で、第3版・両レビュー・関連する契約と実装を照合します。UHD 固有の判断は v4.10.0.0 のソースで確認し、確認できた事実と推論を分けて報告します。
D2 の lookup 集約と新旧 key の instance 単位の分離は妥当です。一方、D16 の根拠「UHD では antenna を timed に切り替えられない」はソースと一致しません。`cold` の採用は維持できますが、その理由と antenna の read-back 契約は補う必要があります。また、D3 の halt からの復帰条件に現行 RM-25 との食い違いがあります。
## 結論

**条件つきで Gate P に進めてよい。** 追加の3判断は維持できるが、下記 P1 の4件を設計上の実装条件として確定してから閉じるべきです。

**P0 はありません。受理済み判断の撤回は不要です。** ファイル変更・build・test・実機操作は行っていません。

以下、`33`＝`plan/phase8/33-channels-and-streams.md`。**VERIFIED** は文書・コードで確認した事実、**INFERRED** はそこから導いた影響です。UHD の参照はすべて **v4.10.0.0** です。

## 追加の3判断

### 1. P6／D2：賛成

Kernel が channel の意味を解釈せず、既存の宣言 lookup だけを広げる最小の変更です。

確認した経路：

| 対象 | 根拠 |
|---|---|
| SB-2 の5位置 | `crates/ezsdr-kernel/src/spec.rs:1171–1199` → `plan/validation.rs:504–555` |
| capability matching／SB-7 | `crates/ezsdr-kernel/src/plan/matching.rs:94–117` |
| SB-45/46 | `plan/validation.rs:125`、`plan/prepare.rs:151`、`session.rs:287` → `plan/coercion.rs:5–34` |
| KC-24 の class・要求値・coerce 後の型 | `spec.rs:1146–1163`、`coordinator/admission.rs:174–204,423–433` |
| schema | `$defs.KeyDecl` の定義は `schemas/vocabulary_descriptor.v1.json:135` の1件 |

**VERIFIED：** 宣言参照は `registry.key_decl` に集約されています。component の `ParamDecl` と `ext.` の既存例外は維持する必要があります。

登録時には、本文の重なり拒否を**同じ Vocabulary 内も含めて**実装してください。異なる indexed 位置どうしでも具体 key が交差する例、不正な位置・prefix 内指定・範囲外指定を拒否するテストが必要です。また、完全一致の探索で indexed 宣言の裸の `decl.key` を受理してはいけません。

これは D2 の変更要求ではなく、`33:71–78` の成立条件を固定する確認です。

### 2. D16：`cold` に賛成。ただし根拠と read-back を補完

Basic/LF の antenna は単なる端子選択ではなく、実数／複素の frontend 接続も変えます。区切って設定し直す `cold` は妥当です。

- RM-21／RM-25 の既存の cut・ready・新 SampleClock を再利用できる。
- Stop 済み RX は再開せず、stream 外の channel は値だけ変更する。
- D7 の antenna 検査と D8 の board 内拘束は、変更後の設定にも適用する。
- Mock も同じ timeline と拒否規則を使う。

ただし、**「UHD の antenna 切替は timed command ではない」という理由は訂正が必要**です。後述 P2-1 のとおり、X300 の該当 register 操作は command time を継承します。`cold` を選ぶ理由は「全対象 board に切替完了時刻・過渡応答まで保証せず、接続変更を segment 境界として扱う」で十分です。

antenna の read-back と不一致時の拒否は P1-2 として残っています。

### 3. 用語：賛成

root を instance 全体、`t<i>` を timekeeper とする判断は整合しています。

- Vision §8 はすでに `Device (or DeviceSet)` と、複数 motherboard を持つ1 handle を想定しています。  
  `design/vision/02-spec-binding-and-compilation.md:39–54`
- Session は binding ごとに root kind の resource を1つ導出します。  
  `design/03-spec-and-binding.md:208,233–243`
- SB-34 は sub-resource binding を**許可**する一般契約であり、radio が timekeeper 単体を実験資源として提供する義務ではありません。  
  同 `:292–298`
- coherence の所有単位も Provider instance のままです。  
  `design/vision/05-coherence-calibration-and-runtime-semantics.md:20–37`

**VERIFIED：** 8b の「複数台を1 instance が所有する」方向と矛盾しません。時刻関係と跨 timekeeper stream の alignment を8bへ残すのも妥当です。

## 前回 R1〜R4 の解消状況

| 指摘 | 判定 |
|---|---|
| **R1：移動の ready・停止状態** | **大部分解消、一部未解消。** 各 clock への cut の丸め、移動元解放を含む ready、両 line の共通実装、両側 held burst の取消しは `33:89–97` に入りました。ただし「halt は membership 更新で復帰しない」が RM-25 と食い違います。P1-1。 |
| **R2：ADC 利得の分配** | **設計方針は解消。** RF 段の格子上の値なら、高優先度 RF 段へ全量、ADC へ0が配分されます。ただし「ADC を触れない」は不正確で、実際には各段の setter が呼ばれ、ADC にも0を書きます。[gain_group.cpp:98–156](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/utils/gain_group.cpp#L98-L156)。開くときの範囲照合は P1-3。 |
| **R3：能力・構成・成立関係** | **計画として概ね解消。** `33:240–247` が3者と失効・較正範囲を分離しました。ただし「timed tune による再現可能な offset」と「較正済みで既知の位相差」は記録でも区別してください。Basic/LF の値は、現段階では実証済みとは言えません。 |
| **R4：段階移行** | **解消。** 別 builder、layout の有無による instance 単位の新旧排他、最終段階での旧経路削除が `33:258–261` に明記されています。旧経路の tree・port・target・共有 API を切替まで維持する実装レビューは必要です。 |

Opus の指摘も、**P0-1 の共有 ADC による更新不能は解消**しています。D14 は既存の target-clock 選択と適合し、coerce は stream target でなく matched root に対して行われるため、D1 の全設定判定を壊しません（`coordinator/pipeline.rs:1011–1039`、`coordinator/admission.rs:358–365`）。

D15 の thread・共通 lock も妥当です。ただし既存 UR-25 の「device call 中は line lock を保持しない」は継承してください（`design/18-uhd-radio.md:247`）。D9 の1列 queue は保守的な直列化として維持できますが、非 streaming 状態の扱いには次の不足があります。

## 新しい指摘

### P1-1：D3 の halt／空 stream の復帰規則が矛盾している

**対象：** `33:85,96`

**VERIFIED：**

- D3 は membership 更新を `cold` としながら、「channel なし・halt は membership 更新だけでは再開しない」としています。
- RM-25 は、halt からは次の `cold` または `start_rx` で復帰すると定めています。  
  `design/07-radio-model.md:254`
- 現 timeline も成功した `Cold` で halt を解除し、on かつ channel 数が正なら segment を開始します。  
  `crates/ezsdr-radio/src/timeline.rs:320–335`

**INFERRED：** 空だが on の移動先へ最初の channel を入れても開始しない読み方と、通常の enable と同様に開始する読み方が併存します。halt 後の再試行も Provider 間で割れます。

**必要な補完：** Stop による off、空だが on、halt を分ける。membership の `cold` が通常の RM-25 に従うのか、例外なら何で復帰するのかを明記してください。

### P1-2：D16 に antenna の read-back・不一致時の扱いがない

**対象：** `33:193`、UR-12 の改訂範囲

**VERIFIED：**

- 現 `Applied` は rate・freq・gain だけです。  
  `crates/ezsdr-radio-uhd/src/device.rs:50–59`
- wrapper は antenna を set するだけで、get していません。  
  `crates/ezsdr-radio-uhd/src/uhd.rs:708–737`
- Manifest も antenna は claimed だけです。  
  `crates/ezsdr-radio-uhd/src/provider/core.rs:459–460`

**INFERRED：** 「新しい clock で再開したが、実際は旧 antenna／旧 A・B・AB 接続」という失敗を検出できません。RF envelope の判定と適用状態も離れます。

**必要な補完：** antenna を read-back・記録し、不一致時は再開せず RM-25 の拒否へ接続する。FakeDevice と Mock の fault にも対応させてください。UHD の getter は設定 property の読出しであり、物理スイッチの独立測定ではない点も区別が必要です。

### P1-3：D5 の開くときの gain 範囲照合が D8 と接続していない

**対象：** `33:134,153`

**VERIFIED：** 空 gain name の RX 範囲は aggregate です。

- RF 0〜31.5 dB ＋ ADC 0〜6 dB → aggregate 0〜37.5 dB。
- Basic/LF RX は RF 段がなく、aggregate は ADC の0〜6 dB。
- D8 が公開する範囲は、それぞれ0〜31.5 dB と `{0}`。

根拠：[gain_group.cpp:66–83](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/utils/gain_group.cpp#L66-L83)、[X300 gain group 登録](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L1718-L1739)。

**INFERRED：** D5 の「範囲を description と比べる」を aggregate にそのまま適用すると、正しい装置を不一致として拒否します。

**必要な補完：** RF stage 名で比較するのか、取得した各段から公開範囲を導くのかを確定する。Basic/LF の「RF stage なし＝公開利得 `{0}`」も明記してください。aggregate への設定自体は維持できます。

### P1-4：D9 の「run していない」が TX と release 済み command を覆っていない

**対象：** `33:171–175`

**VERIFIED：**

- RM-21 の `runs` は RX に定義されています。TX は channel があれば clock を持ち、Stop も clock を終わらせません。  
  `design/07-radio-model.md:235`
- UR-24 は release 済み command を後から取り戻せません。  
  `design/18-uhd-radio.md:245`
- UHD の問題の条件は、論理的な clock／membership ではなく、DDC/DUC を packet が通ることです。  
  [usrp_x3x0.dox:861–863](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/devices/usrp_x3x0.dox#L861-L863)

**INFERRED：** TX が channel と clock を持つだけで burst を送っていない場合や、release 後に Stop／cold cut が入る場合は、本文の判定だけでは安全性が決まりません。

**必要な補完：** TX の idle／burst 間、release 済み command、stream 再開前の deferred 値適用順を定義してください。「効く時点の状態を見る」だけでなく、packet が止まっても安全な発行条件が必要です。危険な実機試験ではなく、まず FakeDevice の停止する queue で固定するべきです。

### P2-1：D16 の「antenna は timed 不可」は原典と不一致

**対象：** `33:194`

**VERIFIED：**

- Basic/LF の antenna は frontend 接続 register を書きます。  
  [x300_radio_control.cpp:296–312](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L296-L312)
- その frontend は timed WB interface を使い、register write は command time を渡します。  
  [同:142–148,203–206](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L142-L206)、[reg_iface_adapter.hpp:79–82](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/rfnoc/reg_iface_adapter.hpp#L79-L82)
- UBX の antenna も GPIO・CPLD write を行います。  
  [db_ubx.cpp:842–871](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_ubx.cpp#L842-L871)

したがって、**timed register 操作が可能であることは確認済み**です。一方、全 board の RF 切替完了・整定時刻の保証までは確認していません。`cold` は維持し、理由だけ訂正してください。

### P2-2：CBX にも同じ full-duplex／RX2 の記載がある

**対象：** `33:155`

**VERIFIED：** 引用元は CBX にも UBX・OBX と同じ規則を記載しています。  
[dboards.dox:309–311](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/devices/dboards.dox#L309-L311)

UBX・OBX を文書根拠で保守的に制限するなら、CBX だけ bench 待ちにする理由はありません。同じ扱いにそろえるべきです。

### P2-3：第3版に古い未決事項が残っている

**VERIFIED：**

- `33:5,48` は追加判断をまだ owner 確認待ちとしています。
- `33:266,273` は ADC 拘束／分配を未決のように残しています。
- 実際の受理は `plan/phase8/00-overview.md:152–154` にあります。

Gate P の記録では同期してください。

## 確認できなかった事項と検索

| 確認対象 | 実行した検索・結果 |
|---|---|
| 独立した KeyDecl lookup の抜け道 | `crates/ezsdr-kernel/src` に `key_decl\|KeyDecl` と `\.keys\b\|key_decl\(`。registry 外で宣言一覧を直接探索する別経路は確認できませんでした。 |
| KeyDecl の schema 定義 | `schemas` に `"KeyDecl"\s*:`。`vocabulary_descriptor.v1.json` の1件。 |
| antenna read-back 契約、release 後の扱い、TX の run 定義 | `33` に `antenna.*(read\|back\|拒否)\|read.*antenna\|WrongAntenna\|get_rx_antenna\|get_tx_antenna\|STOP.*release\|released\|発行済\|送信済\|burst.*run`。該当する具体規則は確認できませんでした。 |
| Basic/LF の実測 phase 保証 | `33` の `phase\|deterministic\|bench` に相当する記述を照合。今後の bench 方針はありますが、実測結果ではありません。 |

**推奨は、3判断を維持し、P1-1〜4を計画に明記して Gate P を閉じることです。Kernel の汎用 transaction、block 別 queue モデル、別の resource モデルへの差し戻しは不要です。**
