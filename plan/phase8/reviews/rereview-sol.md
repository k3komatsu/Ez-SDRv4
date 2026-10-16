読み取り専用で、前回13件を第2版・現行契約・UHD 4.10 の根拠と照合します。受理済み判断は前提とし、未解消点と段階移行の問題を切り分けます。
前回の帯域上限の P0 は、「合計 ≤ 1本」によって解消しています。一方、channel 移動時の再開準備時刻と、旧・新 key を並置する期間の Provider 契約には詰めが残ります。UHD の ADC 利得も、共有範囲だけでなく aggregate gain の分配規則まで確認できました。
## 結論

**条件つきで Gate P に進めてよい。** 受理済みの D1・D9・D13 は維持できるが、channel 移動の再開条件、ADC 利得の分配、coherence の有効性、段階移行の契約を明確にする必要がある。

**新しい P0 はない。** ファイル変更・build・test・実機操作は行っていない。

以下、**VERIFIED** は文書・コード・UHD 4.10 原典で確認した事実、**INFERRED** はそれに基づく問題の推定。

## 前回13件の解消状況

`33` は `plan/phase8/33-channels-and-streams.md`、`overview` は `plan/phase8/00-overview.md`。

| 前回 # | 判定 | 根拠 |
|---|---|---|
| 1：XG の帯域上限 | **解消** | `33:149–157`。方向別合計を1本分に制限すれば、どの link に偏ってもその部分集合は合計を超えない。§59 は厳しい Mock を許す（`design/vision/10-implementation-path-and-testing.md:105`）。実際の持続帯域は BP4c で確認する。 |
| 2：channel 移動 | **一部** | `33:83–92` に共通 instant・handover・両 halt・mapping 記録を追加。ただし両 stream 間の **ready の依存**、Stop 済み stream、拒否後の再計画がまだ曖昧。指摘 R1。 |
| 3：selector の入れ子 | **解消** | `33:108–119` は scalar と scalar list のみ。SB-4（`design/03-spec-and-binding.md:150`）で表現できる。Provider 固有部分との境界は下記補足。 |
| 4：timekeeper／instance の所有 | **解消** | `33:31–48,165` に container・`t<i>`・instance 内通し番号を定義。8b の時刻関係とは切り離せている。 |
| 5：D1 と受理文の差・K2 の理由 | **解消** | `33:54–63` に差と訂正を明記。owner の再受理は `overview:146`。 |
| 6：番号付き key の成立条件 | **解消** | `33:67–74,218` に文法・重複拒否・非具体 key 拒否・policy・不存在番号・各入口・schema を記載。 |
| 7：静的 coerce と runtime | **解消〔方針〕** | `33:159–161` が静的判定と投影・queue・release・拒否後処理を分離。queue footprint の値は明示的な測定残件（`:227`）。 |
| 8：ADC・clock・未確認の排他 | **一部** | clock の共通拒否と推測の排他撤回は解消（`33:123,141–144`）。ADC の aggregate gain の分配規則は未確定（`:226`）。指摘 R2。 |
| 9：aligned／coherent・CBX | **一部** | 区別と CBX の `random` は解消（`33:47,182,198–199`）。追加された D13 は basis と validity の実装上の接続が不足。指摘 R3。 |
| 10：受信 antenna の安全検査 | **解消** | `33:135` に RX の検査と固定論理 TX 番号を明記。 |
| 11：timed capability と admission | **解消** | `33:128–131` が能力情報と admission を区別し、将来機種の設計課題と RS-52 の訂正を記載。 |
| 12：各段階で緑 | **一部** | 改名の同時更新・最終切替は改善（`33:209–214`）。旧 key を残すだけでは旧 Provider／client 契約を維持できない。指摘 R4。 |
| 13：測定の適用範囲・bench | **解消** | `overview:82,115–118,126` に適用条件・BP3 の修正・混在同時運転・XG・再 open・送受同時・未測定範囲の扱いを追加。 |

## 指摘

### R1 — P1：D3 は「共通 effective instant」と「物理的に再開できる時刻」をまだ接続していない

**対象：** `33:85–92`

**VERIFIED：**

- 現行 timeline の再開計算は、その stream 自身の前の cut・旧 rate・receive call・allowance を使う（`crates/ezsdr-radio/src/timeline.rs:376–395`）。
- D3 は移動元の解放後に移動先が取得すると定めるが、移動先の **ready に移動元の解放条件を含める**とは書いていない。
- 現行契約では、Stop 済み RX に cold change を適用しても再開しない。拒否後は記録上の設定を維持し、後続を再計画する（`design/07-radio-model.md:235,251–254`）。

**INFERRED：** 遅い移動元と速い移動先に、それぞれ従来の RM-25 を適用すると、移動先 origin が移動元 streamer の解放より早くなる場合がある。共通 effective instant だけでは防げない。

**必要な追記：**

- RM-16 の cut は、共通 instant から**各旧 clock の sample index に個別に丸める**。同じ instant と同じ sample instant は別。
- 移動先 ready に、移動元の停止・解放と必要な設定の完了条件を含め、Mock も同じ項を使う。
- Stop 済み／空／halted stream は、membership 更新だけで無条件に再開しない。
- 一方の拒否では両方へ拒否を反映し、記録上の membership・後続計画・再開条件をそろえる。
- TX の取消しは移動元だけでなく、**移動先の旧 segment に属する held burst**も扱う。古い clock／mapping の burst を新しい並びで再解釈しない。

これは D3 の撤回ではなく、既存 RM-16・RM-25 を2本へ広げるための補完である。

### R2 — P1：ADC を分けて持つ方針は正しいが、`gain_db` から各段への分配が未確定

**対象：** `33:141,226`

**VERIFIED、UHD v4.10.0.0：**

- RX gain group は RF 段を優先度1、`ADC-digital` を優先度0で登録する。  
  [x300_radio_control.cpp:1718–1739](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L1718-L1739)
- aggregate gain は各段の合計で、設定時は高優先度から配分する。  
  [gain_group.cpp:66–95,98–157,177–185](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/utils/gain_group.cpp#L66-L185)
- ADC は **0–6 dB、0.5 dB step**で、A/B 両方へ同じ値を書く。  
  [x300_adc_ctrl.cpp:78–86](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_adc_ctrl.cpp#L78-L86)
- 現行 UHD wrapper は空の gain name、つまり aggregate gain を設定・取得する（`crates/ezsdr-radio-uhd/src/uhd.rs:702–705,724–737`）。

**INFERRED：** description 内で RF／ADC を分けるだけでは、「同じ slot の要求が両立する」の判定と実機への設定が一致する保証がない。

**必要な確定：** `gain_db` の意味、UHD と一致する分配・丸め、共有 ADC の競合判定、全設定後の read-back を spec に書く。Basic/LF RX は ADC のみ、TX は DAC gain なしを区別する。ここは bench 待ちではなく原典から先に固定できる。

### R3 — P1：D13 の3段を維持しつつ、能力・実際の共有・coherence の有効性を分ける必要がある

**対象：** `33:182–200`

**VERIFIED：**

- Vision の CoherentGroup は `basis` だけでなく、sample alignment state、phase calibration state、coherence validity を持つ（`design/vision/05-coherence-calibration-and-runtime-semantics.md:23–37`）。
- D13 の `lo_group` は「共有**しうる**」集合であり、実際に共有中である宣言ではない。
- UHD は timed tune 後の位相 offset も、LO 周波数によって異なり、温度などで drift し、周期的 calibration が必要とする。  
  [sync.dox:165–177](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/driver_usage/sync.dox#L165-L177)
- HBX の共有 LO 配線にも経路遅延による phase offset がある。  
  [hbx.dox:107–124](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/devices/hbx.dox#L107-L124)

**INFERRED：** phase enum と `lo_group` だけから現在の known-phase 保証を導くと、「共有可能＝共有中」「timed tune 対応＝位相差既知」になり得る。

**必要な補完：**

- **aligned・coherent・shared LO の採用は維持**する。
- `lo_group`、選択された source/export/layout、実際に成立した basis を区別する。
- coherence の成立・失効を、timed tune の結果、alignment／clock loss、必要な calibration validity と結び付ける。
- 「較正は一度でよい」（`:186`）は、**どの周波数・経路・状態の範囲でか**を限定する。無条件の恒久保証にはしない。
- Basic/LF の `deterministic` は、現在の INFERRED 表示を維持し、DSP tune と timed restart を含む位相確認を bench 項目に追加する。LO がないことだけでは、その確認の代わりにならない。

これは受理済み3段の撤回ではなく、Vision の `basis` と validity に接続するための条件である。

### R4 — P1：「旧 key を残す」だけでは 1b-2〜1b-4 の緑を保証できない

**対象：** `33:209–214`、`overview:96–98`

**VERIFIED：**

- 1b-3 は Mock を新 key で動かし、server・Python・fixture の切替は1b-5。
- 現行 `effective` は旧10 key が契約で、実際に完全一致のテストがある（`design/07-radio-model.md:97–99`、`crates/ezsdr-mock-radio/tests/mock_radio.rs:604–607`）。
- tree builder と timeline の型も両 Provider が使う（`crates/ezsdr-radio/src/device.rs:240–267`、`crates/ezsdr-radio/src/timeline.rs:10–17`）。

**INFERRED：** 新 Mock が新 key だけを返す、旧 port／Action target を先に替える、共有 API を置き換える、のいずれでも1b-5以前に既存経路が壊れる。

**必要な追記：**

- 最終切替までは旧 key・port・target・共有 API をどう動かすかを明記する。内部の正規化を使うなら、新旧の同時指定と `effective` の規則も決める。
- 一時的な二重契約を作らないなら、Provider／client の切替部分を同じ gate にまとめる。
- `overview:96` の「値は変えない」は、`33:209` の rate grid 修正・CBX `random`、共通 description 化で変わる既知の差を例外として同期する。

## D5・D9 の補足

- **D5 の平坦 selector は SB-4 上、問題ない。** `id` は物理 frontend の識別子ではなく、instance／tree root `<r>` の名前として扱う。現行両 Provider もそうしている（`crates/ezsdr-mock-radio/src/lib.rs:237–243,281–285`、`crates/ezsdr-radio-uhd/src/provider/mod.rs:116–120,177–181`）。
- 共通 parser は layout・master clock・image・links の形と整合を担当し、UHD の `args` 等と Mock の fault／pattern 等は各 Provider が担当する。**同じ selector 全体を受け付ける、という意味にはしない。**
- **D9 の受理を覆す理由はない。** 「合計 ≤ 1本」は link 偏りに対して安全側。ただし1本分の数値そのものと送受同時負荷は、引き続き測定対象である。

## 検索したが、具体化を確認できなかった事項

`plan/phase8/33-channels-and-streams.md` に対して以下を実行した。

| 検索 | 結果 |
|---|---|
| `ready\|準備\|解放\|止ま\|停止\|Stop\|halt\|read.back` | 両 stream 間の ready 依存、Stop 状態を保つ具体規則は確認できなかった。 |
| `互換\|alias\|変換\|旧.*新\|並置\|並べ\|競合\|正規化\|移行` | 旧 key の並置方針はあるが、移行期間の実行・競合・`effective` の規則は確認できなかった。 |
| `calibration\|較正\|校正\|validity\|有効\|basis\|clock loss\|ALIGNMENT` | D13 の説明1行だけで、成立・失効・calibration validity の規則は確認できなかった。 |

**推奨：R1〜R4 を計画上の実装条件として確定して Gate P へ進む。受理済みの基本判断の再選択や、Kernel の汎用 transaction 追加は不要。**
