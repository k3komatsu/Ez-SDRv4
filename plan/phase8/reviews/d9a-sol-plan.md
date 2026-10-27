# D9a の試験計画：Sol 6.1 の計画レビュー（2026-10-27）

計画は [d9a-test-plan.md](d9a-test-plan.md)（改訂 1〜3 の節がそれぞれの回に答える）．

## 1 回目
## 独立に挙げた失敗系列

計画・指示書 §3 を読む前に、次を挙げた。

1. Stop／halt／channel 離脱中の多数の RX 更新が DDC に溜まる、または再開時に失われる。
2. future segment の configure 前・実行中・完了後の更新が、snapshot と構成記録の隙間に落ちる。
3. destination の owner が source の解放を待つ間、untimed 更新が source の古い sample に早く効く。
4. `e == origin`／`e == cut`、release と cut／拒否／loss の交差で、配送先や記録が変わる。
5. fake が host の `recv` を FPGA の sample 通過と混同し、偽の満杯や過去の sample への遡及適用を起こす。
6. TX の timed→untimed→timed continuation、streamer の交換、FIFO overflow が F3′／S2 を隠す。

## 計画との比較

Stop・遠い segment・configure 済み・snapshot 公開前の競合・gain の不変性・S2／F3′を分けた点はよい。特に `targets()`／`channel_live()` を記録用に残し、R1 の物理配送だけを変える方針は、最小変更として妥当。

ただし、**`Holder::Owner` を configure 完了の代用品にする案は安全でない**。また R5 は、実機の内部時間・rate・複数 queue を単純化しすぎており、その模型を検査するテストだけでは production の正しさを保証できない。

## 判定：FAIL — major 6、minor 3

### Major 1 — owner の claim は「その chain がもう流れていない」を保証しない

計画 §0.4・§3 の根拠が成立しない。

`owner_configure()` は claim を取った**後で**、他 stream の channel 解放を待つ（`provider/core.rs:1239–1268`）。したがって destination が `Holder::Owner { seq }` でも、source はまだ受信中でありうる。

具体例：

- source の cut = 100 ms、destination の origin = 160 ms。
- destination owner は channel の解放待ち。
- gap の更新 `e = 101 ms` を 98 ms に release。
- `flows(e)` は偽だが destination の holder は Owner。
- 提案どおり untimed に渡すと、source の 98–100 ms の sample を壊す。

**変更：** この系列を実際の `await_release` と gate で追加する。「release 時の untimed 呼出しなし、source は旧値、destination は新値」を検査する。configure **済み**の最適化は残せるが、claim だけで「configure 中／完了」と判定しない。snapshot の隙間は、既存の projection lock と構成完了の公開を同期させる案を優先して再検討する。

### Major 2 — RX の遅延評価が物理的な sample 通過と一致しない

計画 §1:40 は「`rx_recv` を回せばよい」とするが、実機は host が読まなくても sample が DDC を通る。

走行中に host の受信を一時停止し、期限を過ぎさせながら 40 回更新すると、提案 fake は満杯になりうる。さらに、後から書いた値を、未読の古い sample に遡って適用する危険もある。

**変更：** queue の進行を `rx_recv` の呼出し回数ではなく、device 時刻と実際の start／stop 区間から決める。遅延計算自体は構わないが、書込み・満杯判定・stop 前にも必要な進行を反映する。

追加テスト：

- host が読まない間も、流れている DDC は期限の来た entry を消費する。
- 後の更新／configure は、それ以前に生成済みの未読 sample を変えない。

### Major 3 — 「1 sample」の rate が違う

計画 §1:35・R5b は、RX の返却 sample ごとに一つ消費する。しかし DDC の消費点は **decimation 前の入力 sample**（radio rate）。

[`rfnoc_block_ddc.v:298–319`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_ddc/rfnoc_block_ddc.v#L298-L319) は `tick_rate=1`、[`ddc.v:118`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/ddc.v#L118) は入力 sample の handshake で値を消費する。DUC も interpolation 後に timed DDS がある。

200 MS/s→1 MS/s で期限切れの三つを「最初の三つの host sample」で消費する模型は違う。

**変更：** 内部 sample 時刻での消費と、host sample の観測を分ける。非自明な decimation／interpolation で試験する。完全な DSP filter 模擬は不要だが、host rate の FIFO として固定しない。

### Major 4 — FIFO の容量・overflow を、未検証の正確な契約にしている

計画 §1:38・R5i の「32 個で最後の entry の値を置換」は、実機の timestamp/value queue の不整合とは別の模型。

[`axi_tag_time.v:157–188`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_tag_time.v#L157-L188) には timestamp FIFO と追加の出力 FIFO があり、value 側には別の保留 register／FIFO がある。「32」という基礎深さだけで、chain 全体の正確な受理上限は決まらない。「最後には最新値」は前回レビューでも保証しなかった。

**変更：**

- 32 は採用する保守的な fake envelope と、実機の正確な容量を区別する。
- overflow の具体的な対応を source から導出するか、未検証の抽象模型と明示する。
- 「最終値が最新」を必須の hardware 契約にしない。
- 異なる instant／値を使い、どの対応が壊れるかを観測する。

### Major 5 — P4 では production の stop 待ちを判定できない

P4 は `tune_dsp()` だけを使う。しかし確認したい仮説は、production の AUTO/AUTO tune が作る **radio の timed command の後ろで stop が待つか**である。DSP-only command では、その radio queue の前提を作らない。

また P4 の手順には close／reopen がなく、R5f の「streamer close を越えて残る」の裏取りにもならない。

**変更：** DSP-only の対照に加え、実際に DSP 値も変わる production 相当の AUTO/AUTO tune を使う条件を設ける。呼出し前後・最後の RX sample・再開後の値を記録する。close／reopen の検証は別条件にする。P4 前の RX Ceiling は、確認済みの実害ではなく条件付きと記す。

### Major 6 — 境界・競合の mutation に、対応する観測が不足

- 系列4・5は `e == origin` を指定していないので、`<=`→`<` を殺す保証がない。
- 系列6は **release 後**の cut。判定時に既知の `e == cut` とは別。
- 3b は configure が返った後で止めるため、それだけでは「projection lock を取らない」mutant を殺せない。
- R5g に timed→untimed→timed がなく、途中で誤って `has_time` を復活させる実装を見逃す。

**変更：** それぞれ等値境界、既知の cut、configure 内部との交差、timed→untimed→timed→EOB→次 burst を明示する。mutation 候補には、実際にその差を観測するテストを対応付ける。

### Minor 1 — VH-8 の代替は、対の一致だけでは弱い

gain-only という指摘は正しい。共有 generator を広げることは、この段階の必須条件にはしない。

ただし12bの両側は同じ UHD の記録処理を通る。**両方が同じ間違いでも通る。** stopped、future segment、拒否、stop／loss の `e == S` を、明示した期待行でも検査する。floor・枠・late event も残す。

### Minor 2 — 最初の sample を `dsp_log` の最初の変更点と同一視しない

1c は untimed configure の時刻が origin より前なので、origin に変更 log がなくても正しい。sample の有効値を直接検査するか、origin での状態を履歴から評価する。ログ自身だけを正解にしない。

### Minor 3 — probe と文書の完了条件を補う

P6 の H2 観測には、意図的な途中 starvation と、その後の送信手順を指定する。正常送信で underflow が出なかっただけでは回復挙動を確認できない。

文書一覧には F3′の現存制約、README→`KNOWN_LIMITATIONS.md` のリンクも含める。R4・R2・F7 の修正自体は D9b のままでよい。

---

変更・build・test・Docker・実機操作は行っていない。streamer close 時の FIFO 保持、正確な容量、deployed FPGA との一致は未確認。

## 2 回目
## 判定：FAIL — Major 2、Minor 3

改訂の方向は妥当。前回 Major 2・4、Minor 1・2 は解消。Major 1 の claim 判定も撤回され、snapshot と configure 完了印の同期は改善された。ただし、FIFO の観測と P4 に未解決が残る。

### configure 済み判定の再確認

**「configure は必ず `await_release` の後」は正確でない。** `owner_configure()` が待つのは、`Owner::Open` かつ競合がある場合だけ（`crates/ezsdr-radio-uhd/src/provider/core.rs:1245–1275`）。

ただし、他 stream が channel を実際に保持していれば `held_elsewhere` が検出する。from-none も `held_elsewhere` と、全 origin を対象にした `needed_before(..., i64::MAX)` で制限される（`provider/control.rs:957–959`）。したがって、**同じ channel を他 stream が流している最中に from-none が configure する反例は、HEAD では見つからなかった。**

これは「現在の plan にある、その channel の宛先 segment の印」を読む場合の結論であり、残存する projection のどれかが configured、という判定には適用できない。

## Major 1 — R5m の最初の host sample の期待が、模型自身と矛盾する

`d9a-test-plan.md:48,101` は、期限切れの三つを start、start+1、start+2 tick で消費し、decimation 200 なら「最初の host sample は三つ目の値」とする。一方、同計画 `:39–41` は sample の tick の `freq_at` を観測する。

最初の host sample の timestamp が start なら、その tick の値は一つ目であり、三つ目ではない。HEAD の fake も `rx_start(at)` で `next=at` とする（`src/device.rs:1211–1215`）。実機の decimation/filter 過渡も、単純な三つ目の値とは同一視できない。

**変更：**
- radio-rate の消費は、三つの連続 tick の履歴で検査する。
- 三つ目の値は start+2 以後の明示した観測点で検査する。
- host sample は timestamp と期待値を対応させる。「最初は三つ目」を成立させるために sample 時刻や履歴をずらさない。

前回 Major 3 の rate 修正はよいが、この期待のまま実装すると別の誤模型を固定する。

## Major 2 — P4-B には AUTO/AUTO の DSP 変更を確認する観測手段がない

計画 `:191` は、`tune_dsp` の結果または `apply` の戻りで、AUTO/AUTO が DSP 値を変えたことを確認するとする。しかし：

- `tune_dsp()` は getter ではなく、**DSP を MANUAL で書き換える**（`src/uhd.rs:518–551`）。
- `apply()` は AUTO/AUTO の `uhd_tune_result_t` を取得するが、その結果を外へ返さない（`:875–888`）。戻る総合周波数から LO/DSP の内訳は分からない。

このままでは、必要な DSP entry が存在したと確認できないか、確認操作が実験条件を変えてしまう。前回 Major 5 は未解消。

**変更：** AUTO/AUTO の同じ tune 呼出しから `actual_rf_freq`／`actual_dsp_freq` を非破壊で記録する probe 経路を指定する。前後の DSP 値が異なる条件だけを有効な P4-B とし、作れなければ **INCONCLUSIVE** とする。C 条件も、close 前に未消費 entry があることを記録する。

## Minor 1 — M4 の booking と snapshot の交差は、その説明どおりには作れない

計画 `:92` は、configure 前に book しながら、その値が snapshot に入らない場合を要求する。だが `ColdConfig::book()` と configure は同じ `updates` lock を使う（`provider/control.rs:107–109,126–143`）。

book が先なら snapshot に入り、snapshot が先なら book は configure 完了まで待つ。また同じ control の `give` は book 完了後である。

**変更：** M4 を実現可能な二順序に分ける。configure 中に booking thread が待ち、完了後の release が untimed で当たることを検査する。「snapshot に無い値を失わせる mutant」という説明は撤回する。configure 完了後・印公開前の穴は M3 で検査できる。

## Minor 2 — M5 を実装時の探索に残さない

計画 `:93,129–135` は guard 不要を設計として採用しつつ、「反例が作れたら guard」を実装役に残している。

**変更：** 宛先 segment の選択と印の有効範囲を今決める。後続 cut で configured segment が消える系列、move の往復を固定テストにし、古い `ColdConfig` の印を宛先判定に使わないことを観測する。`projections()` は全 configs を返す（`provider/control.rs:471–473`）ため、単なる configured 印の存在確認では不足する。

## Minor 3 — P6 の starvation を device 時刻で確定する

計画 `:193` の 5 ms sleep だけでは、先行して queue した sample が残り、underflow しない可能性がある。

**変更：** 最後に queue した sample の終端を記録し、device 時刻がそれを越えてから continuation を送る。Underflow が観測できなければ、回復系列は未検証と記録する。

---

変更・build・test・Docker・実機操作は行っていない。FIFO 保持・容量・deployed FPGA との一致は引き続き未確認。

## 3 回目
## 判定：FAIL — Major 1

改訂2で、前回の **Major 2件・Minor 3件は解消**されています。ただし、新設した「最初の segment は deferral」という規則に、値を失う経路があります。

### Major — `by == None` の宛先には、deferral した値を運ぶ configure が残っていない

対象：`d9a-test-plan.md:140–142`

具体例：
1. 初期周波数 v0 で prepare し、十分先の T0 に RX を開始する。
2. T0 より前に control が動き、`now < e < T0` の周波数 v1 を book／release する。
3. `flows(chan,e)` は偽。宛先は最初の segment だが、`by == None` なので新規則は deferral する。
4. T0 では再 configure されず、最初の sample は **v0 のまま**になる。

HEAD の根拠：
- 初期 configure は prepare 中に既に完了する：`crates/ezsdr-radio-uhd/src/provider/mod.rs:535–543`。
- start は未来の T0 を受け付け、T0 を待たず control thread を起動する：同 `:611–619,665,696–707`。
- RX の最初の segment は既に開始済みとして渡される：`src/provider/rx.rs:148–162`。
- `planned()` は `by == None` に settings を付けない：`src/provider/core.rs:582–590`。

`of_record` は control の設定を更新するだけで、装置には当てません（`src/provider/control.rs:693–695`）。したがって「後の configure が運ぶ」という説明では救えません。

**必要な変更：** 初期 segment にも、prepare 済みであることを表す判定と release 時の untimed 適用経路を定義してください。T0 より前の release を制御時計で再現し、timed apply は0、untimed apply が入り、T0 の最初の sample が v1 になる固定試験を追加してください。単なる INFERRED の確認項目には残さないでください。

### 前回5件の確認

- **R5m：解消。** radio-rate の連続3 tick と、host sample の timestamp ごとの期待値が一致しています。
- **P4-B：解消。** probe 専用 `tune_auto` は production と同じ AUTO/AUTO 呼出しから RF/DSP 結果を返します。観測用の MANUAL 書換えは不要です。DSP 不変なら INCONCLUSIVE、C 条件の未消費 entry の記録も明記されています。
- **M4：解消。** M4a/M4b は同じ `updates` lock に沿った実現可能な順序です。M4b が冗長適用を固定する点も明示されています。
- **M5：解消。** 毎回の `planned()` 再読、拒否・channel・origin・非零長・最小originの条件、その宛先自身の印という限定は明確です。M5a/M5b は零長化と往復moveの古い印を検査します。
- **P6：解消。** queued sample の終端を device 時刻で越え、Underflow 未観測時は回復未検証とします。

変更・build・test・Docker・実機操作は行っていません。実機のFIFO保持・容量は引き続き未確認です。

## 4 回目
## 判定：PASS — 前回 Major は解消

改訂3の `is_none_or(|config| config.configured())` は、prepare 済みの初期 segment へ release 時に untimed で適用するため、`now < e < T0` の値を失いません。M6 は timed apply 0、untimed apply 1、FIFO 空、T0 の最初の sample が v1、を観測し、対応する mutation も明確です。

### `settings: None` の全経路

HEAD の `crates/ezsdr-radio-uhd/src/provider/core.rs:589` には二経路あります。

- **`by == None`**：初期 segment。prepare の configure は成功済みです（`provider/mod.rs:533–543`）。
- **`by == Some(_)` だが configs に無い**：prune 済みの過去 segment。`core.rs:574–578` が configs を削除します。RX は segment 終了時、TX は通過済み segment に対して prune します（`provider/rx.rs:355–371`、`provider/tx.rs:199–218`）。未 configure の将来 segment ではありません。

Start・cold・move は streams lock の同じ区間で configs を登録します（`provider/control.rs:433–439,869–885,1039–1049`）。cold の登録を削除するのは booking 不成立時だけです。**未 configure の将来 segment が `settings: None` になる production 経路は見つかりませんでした。**

### 文言の小修正のみ推奨

計画 §3 の「settings が無い＝`by == None`」は厳密には不正確です。上記の **prune 済み過去 segment も None になるが、宛先候補にはならない**旨を追記してください。判定式や経路を増やす必要はありません。

今回の追加に伴う近傍の矛盾・新たな major はありません。

変更・build・test・Docker・実機操作は未実施。判定は指定された計画差分と HEAD の静的確認に限定します。
