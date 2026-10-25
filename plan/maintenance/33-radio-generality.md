# 33 — Radio Model と UHD Module の汎用性の点検

Status: 当てはめと Sol のレビューまで済み，owner の判断待ち．Sol は3回：FAIL（major 6，minor 1）→ FAIL（S1〜S7 は解消，major 1）→ PASS（[記録](33-sol-review.md)）（owner 2026-10-24 の依頼，2026-10-25 開始）．事実：[HackRF](33-facts-hackrf.md)，[bladeRF](33-facts-bladerf.md)，[N3xx と X4xx の差分](33-facts-n3xx.md)（いずれも sonnet-researcher，2026-10-25．取得の道具が行番号を落としたので関数名と引用で示す）．handoff §2 の順番で Phase 8 の後・凍結前に置いた点検の設計メモ．紙の作業は Phase 8 段階 2 の合間に先にしてよい（同依頼）．

## 1. 目的と範囲

凍結（v4.0）の前に，Radio Model（spec 07）と Kernel の時刻・stream の規則（spec 01，02）が X3xx 以外の SDR を表せるかを確かめる．見つけたいのは **Kernel を変えないと表せないもの**で，凍結後には直せない．Vocabulary に足せば済むもの，Module の中で閉じるものは記録して後の段へ回す．

対象：
- **A 別の SDR 系**：HackRF One・HackRF Pro（libhackrf，v3 の `v3/source/device/hackrf.d` が証拠），bladeRF 2.0 micro（libbladeRF）．
- **B USRP の別系統**：N3xx（N310，N320/N321）を UHD 4.10 のソースで読む．X410・X420・X440 は [usrp-family-limits.md](../phase8/research/usrp-family-limits.md) で調べ済みで，ここでは点検表に答えのない項目だけを足す．

範囲外：Module を書くこと（HackRF One → bladeRF 2.0 の Module は，この点検の後の段），RFNoC（8c），複数台（8b）．

## 2. 方法

1. 点検表（§3）：Radio Model が暗に置く前提を，規則の場所と一緒に並べる（orchestrator）．
2. 事実集め：`sonnet-researcher` を1本ずつ，A1 HackRF → A2 bladeRF → B N3xx の順に．点検表の各項目に，VERIFIED / INFERRED と根拠（URL かソースの `path:line`，tag を固定）で答える．見つからないものは実際に使った検索を書く．結果は `33-facts-<device>.md`．
3. 当てはめ（§5）：項目ごとに **合う／Vocabulary に足す／Module で閉じる／Kernel を変える** の4つに分ける（orchestrator）．
4. Sol がこのメモをレビューする（read-only）．
5. owner の判断（§6）：変える必要のあるものを1件ずつ．採ったものは別の段で spec の amendment にする．

## 3. 点検表

「どこ」は今その前提を決めている規則．「問い」は研究者に装置ごとに答えさせること．

### 時刻

| ID | 前提 | どこ | 問い |
|---|---|---|---|
| G1 | Run の primary Root は装置の時刻（timekeeper）で，Authority は装置の時刻を読み，PPS で 0 に合わせる | TM-16a，UR-6，UR-7 | 装置は読める時刻のカウンタを持つか．単位（sample か別の clock か），幅，設定できるか，PPS や trigger で合わせられるか，RX と TX で共通か，stream の開始で 0 に戻るか |
| G2 | 受信の各 sample の時刻は origin + index で正確に決まる（origin は最初の sample を予定した root tick） | TM-13b，RM-25 | 受信の buffer に装置の timestamp が付くか．付かないとき，最初の sample の時刻をどう知るか（timed start があるか，host の時計で推すしかないか，その誤差） |
| G3 | 送信は時刻付きの burst（start of burst に time spec，end of burst）で，target 前の到着は on time，過ぎたら late | RM-13〜RM-15，SC-23〜SC-29 | 送信の buffer に開始時刻を付けられるか．burst の開始・終了の印を持つか．時刻なしなら「受け取り次第送る」だけか |
| G4 | 周波数と gain の変更は `hardware_timed`：装置が指定時刻に実行し，queue（深さ）と release window を持つ | RM-21，RM-27，RM-4 の `radio.timing.*` | 周波数・gain を時刻指定で変えられるか（どちらか，どの範囲で）．queue の深さ．時刻指定できないとき，変更にかかる時間と，その間の sample がどうなるか |
| G5 | `radio.hardware_time` は true だけが実装されている（`channels.rs:358,376`）．false の Provider が `at` をどう扱うかを決めた規則はない | RM-4 | （判断の項目．研究者には問わない） |
| G6 | 受信の欠落は数が分かる：overrun は時刻の跳びと `lost`，「v4.0 の producer はどれも timestamp を持ち，`lost` を必ず知る」 | SC-13，spec 02 §10 issue 2，RM-17，RM-18 | overrun や取りこぼしを報告するか（どこで，何を）．失った sample の数が分かるか．分からないなら，次の buffer の時刻は分かるか |
| G7 | 送信の不足（underflow）と sequence error が非同期の報告で分かる | RM-10 `TX_UNDERFLOW`，UR-28 | 送信側の underflow や遅着を報告するか |
| G8 | 周波数の基準（10 MHz）を失ったら分かる（`CLOCK_LOST`）．PPS は見ない | RM-11，UR-27 | 外部基準入力があるか（周波数），lock や存在を読めるか |
| G9 | 複数台の揃った開始は PPS と装置の時刻で（§61 の4つ目，8b） | §61，`Session.aligned` | 複数台を揃えて始める仕組み（PPS，trigger 入出力，clock 分配）．どの精度で揃うか |

### stream と channel

| ID | 前提 | どこ | 問い |
|---|---|---|---|
| G10 | 全二重：RX と TX の stream が同時に走れる（`radio.full_duplex` は true だけ実装）．duplex board の制約は RM-31 の antenna 規則だけ | RM-4，RM-31 | 同時に送受信できるか．できないなら，切り替えにかかる時間と，切り替えの間に何が起きるか |
| G11 | stream は同じ rate の channel の集合で，channel ごとに周波数・gain・antenna を持つ．stream の間の制約は `Description::violations`（RM-31，RM-32）で判じる | RM-3，RM-21，RM-27，RM-30 | channel の数（RX/TX）．channel ごとに独立に設定できるもの，共有されるもの（LO，rate，gain，antenna）．RX と TX の rate は独立か |
| G12 | rate は離散の格子（`AnyOf`，X3xx は master clock ÷ decimation），または整数 Hz の範囲 | RM-8，RM-29，`device::Grid` | 設定できる rate の集合（範囲，刻み，分数で作るか）．装置が丸めるか，丸めた値を読み返せるか．rate とともに変わるもの（filter の帯域） |
| G13 | 周波数と gain は channel ごとに一つの範囲と一つの刻み．gain は一つの数（UHD の aggregate） | RM-8，RM-27，RM-29 | 周波数の範囲と刻み（LO の分解能，丸め）．gain の段（名前，範囲，刻み，on/off の amp）．合計の gain を一つの数で表せるか．周波数で範囲が変わるか |
| G14 | 標本の形式：Kernel の stream は `ezsdr.stream.cf32`．wire 上の形式は Module の中（`radio.perf.wire_bytes_per_sample`） | SC-5，RM-7，RM-13 | wire 上の形式（sc8，sc16，Q11 など），host への転送路（USB 2/3，Ethernet）と実効の throughput |
| G15 | 位相：channel ごとに retune 後の位相の振る舞いを3値で宣言する．channel 間の coherence は宣言しない（同じ LO を共有するかは書けない） | RM-9，Vision §26 | retune 後の位相は決まるか．複数 channel は LO を共有するか．timed tune で位相が揃うか |
| G16 | 経路の遅延は channel ごとに固定の sample 数 | RM-23 | 送受信の遅延（固定か，rate や設定で変わるか）．資料に値があるか |
| G17 | 送信の繰り返し（`start_repeat`）は Module が host から繰り返す．上限と長さの倍数を capability で宣言 | RM-13，UR-22 | 装置に繰り返し送信の機能があるか（不要なら「ない」でよい） |

### 安全，資源，構成

| ID | 前提 | どこ | 問い |
|---|---|---|---|
| G18 | RF の安全枠は送信の周波数帯，gain の上限，channel ごとの送信可否，antenna port | RM-19，RM-33 | 送信電力の範囲．安全に関わる設定で今の枠にないもの（antenna port の給電＝bias tee，amp の on/off など） |
| G19 | layout の selector は X3xx の形（`master_clock_hz`，`image`，`links`，`"<timekeeper>:<slot>:<board>:<frontend>"`，7種の board） | RM-28 | 装置の構成を表すのに要るもの（型番，front end の種類，拡張 board，firmware/FPGA の版） |
| G20 | TimingEnvelope は装置に1つ：最小の command lead，起動の遅れ，queue の深さ，release window，overflow の再開の間，方向ごとの開始の lead | RM-6 | それぞれに当たる値，または値がないこと（資料，ソースから分かる範囲で） |
| G21 | 装置を失ったことが分かる（`DEVICE_LOST`） | UR-29，MA-9a | 切断や異常をどう知らせるか |
| G22 | 一つの装置に一つの root（timekeeper）．装置の識別は device string | UR-6，UR-7，usrp-family-limits X11 | 一つの装置の中に別々の clock で動く部分があるか |
| G23 | timed command は sample の格子で適用される（UC-6：格子でしか適用できない装置は次の sample の瞬間へ進める） | UC-6 | command の時刻を比べる格子は sample の格子と同じか（Sol の S6 で追加．研究者には問うていない．事実は X440 だけ，§5） |

## 4. 調べる装置と資料

| 本 | 装置 | 資料（tag を固定） | 備考 |
|---|---|---|---|
| A1 | HackRF One，HackRF Pro | greatscottgadgets/hackrf（libhackrf，firmware）の最新 release tag，公式文書，`v3/source/device/hackrf.d` | Pro は何が公開されているかから確かめる．v3 は `hackrf_set_hw_sync_mode` を使っている（`v3/source/device/hackrf.d:92`） |
| A2 | bladeRF 2.0 micro（xA4/xA9） | Nuand/bladeRF の最新 release tag（libbladeRF），公式 wiki，AD9361 の資料 | sync API の metadata（timestamp，burst の印），scheduled retune，trigger |
| B | N310，N320/N321 | EttusResearch/uhd `v4.10.0.0`，Ettus の manual | MPM，master clock rate，timed tune，daughterboard（Magnesium，Rhodium），複数 timekeeper の有無 |

## 5. 当てはめ

分類：**合う**（今の規則と型のまま，Module が値を入れる），**語彙**（`radio` Vocabulary に key・規則・description の形を足す．凍結後も足せる），**Module**（Module の中で閉じる），**Kernel**（Kernel の規則か型を変える．凍結前に決める）．装置の略：H＝HackRF One/Pro，B＝bladeRF 2.0 micro，N1＝N310，N2＝N320/N321，X4＝X410/X420/X440．事実の出典は各 facts ファイルの同じ G 番号．

### 5.1 項目ごとの分類

「合う」のうち INFERRED の事実に頼るものは，その条件を行に書く（Sol の S7）．Module を書く段で確かめる．

| ID | 分類 | 装置ごとの事実と当てはめ |
|---|---|---|
| G1 | 合う（条件つき．H は K2） | N1・N2・X4 は X3xx と同じ timekeeper（64 bit，MCR の tick，今または次の PPS で設定）．B は sample の rate で進む 64 bit のカウンタ（RX・TX 別，同じ clock）で，設定できず PPS もないので epoch は `Arbitrary { set_by }`（TM-3）．条件：B の RX と TX のカウンタが同じ値か（INFERRED），trigger はカウンタを 0 にしない．H は時刻のカウンタがない（K2） |
| G2 | 合う（条件つき．H は K2） | N1・N2・X4 は timed start．受信 packet の timestamp は X3xx と同じ RFNoC の道から INFERRED．B は metadata 形式で message ごとに先頭の timestamp，timed start は host が要求時刻まで捨てて作る（Module）．H は timestamp も timed start もない（K2） |
| G3 | 未決（B）／K2（H） | B は burst の開始時刻と終了の印を持つ．FPGA に着いた遅着は報告なしにすぐ送られる（`fifo_reader.vhd`，取得の道具の要約）．host 側の判定（`BLADERF_ERR_TIME_PAST`）は判定の後の転送の遅れを防げないので，RM-14 の「target ちょうどに始まるか，late として扱う」を Module だけで守れるとは言えない．B の Module を書く段で確かめる．H は時刻付きの送信がない（K2） |
| G4 | **Kernel になりうる（K1）** | 周波数・gain を時刻指定で変えられない装置がある：N1（AD9371 は RPC で untimed，約 88 ms，INFERRED），H（すべて），B の gain と rate（周波数は scheduled retune で timed，quick tune の profile が必須），X410・X440（RFSoC の NCO が untimed）．値で変わる装置もある：X420（500 MHz 以上だけ timed，FAMILY），N2（spur reduction を有効にすると 450 MHz をまたぐ tune が untimed）．Kernel が要るかは，今の key のまま timed の更新を残すかで決まる（判断1） |
| G5 | 語彙（K2 と一緒に） | `radio.hardware_time` が false の Provider が `at` をどう扱うかの規則がない |
| G6 | 合う（条件つき．H は K2） | B は overrun の後の timestamp の跳びから失った数を出せる（RM-17 の `lost` ＝跳び）．条件：`sync_rx` の overrun の戻り方は取得の道具の要約に頼っており，跳びの出し方は INFERRED．N1・N2・X4 は X3xx と同じ UHD の streamer．H は失った回数と最長の長さだけで，位置も合計も分からず，欠落の間カウンタも止まる（K2） |
| G7 | **Kernel の約束に関わる（K2 で扱う）** | SC-25 は underflow を「起きた時刻を持つ `TX_UNDERFLOW`」と定める Kernel の規則で，Provider の spec では外せない（Sol の S4）．UHD Provider は報告に時刻がないとき Authority の `now` を使う（SC-25 の Test 行）．H は host が M0 の状態を poll すれば回数は分かるが，起きた時刻は分からない．B は FPGA の underflow の数が外に出ておらず（`tx.vhd` で `open`），host は知りようがない |
| G8 | 合う | どれも lock か存在の poll（B は PLL の lock bit，H は `hackrf_get_clkin_status`，N1 は daughterboard の LMK）．`CLOCK_LOST { reference: frequency }` で表せる |
| G9 | 合う（8b） | B・H は PPS ではなく trigger で揃える（B はレベル感応，master が2台で pin を壊す警告．H は1 sample 未満と文書）．trigger の edge を epoch にした root は `Arbitrary { set_by }` で表せる．ただし B の trigger は sample を止めるだけでカウンタを揃えない．複数台を1 instance・1 root にする 8b の判断1が当たるかは 8b で確かめる |
| G10 | 語彙 | H は半二重．「一つの装置で RX と TX の stream が同時に channel を持たない」を `Description::violations` の規則にする（RM-31 と同じ形）．`radio.full_duplex` は false で宣言する |
| G11 | 語彙 | 共有の拘束：N1 は board 内の2 channel が方向ごとに LO を共有（一方の周波数を変えると相方も変わる），B は方向ごとに LO を共有し RX と TX の rate が連動（INFERRED），H は RX・TX で周波数と rate を共有．どれも `violations` の規則（design 33 D8 の channel 間の拘束の形）で表せる |
| G12 | 語彙（D1 は方針） | B は 520 834〜61.44 MHz の 2 Hz 刻み（`device::Grid` に刻みが要る），H は整数 Hz に丸める分数合成．B の時刻のカウンタは sample の rate で進むので，Run の中で rate を変えたときに装置の時刻をどの root に写すかが問題になる（判断3）．N1（122.88／125／153.6 MHz）・N2（200／245.76／250 MHz）の MCR は変えると再初期化で，X3xx の `master_clock_hz` と同じ selector の値になる |
| G13 | 語彙 | B の RX と ZBX は gain の範囲が周波数で変わる．H は段ごとに刻みが違う（LNA 8 dB，VGA 2 dB，TX VGA 1 dB，amp の on/off）．RM-8 の「channel ごとに一つの範囲と刻み」を，周波数の帯ごとの表に広げるか段の key を足す．`coerce` は構成全体を見るので，周波数に依る範囲も純粋な関数のまま書ける |
| G14 | Module | H は sc8（Pro は 4 bit・16 bit の mode もあるが host の API が未確認），B は SC16 Q11／SC8 Q7，N1・N2 は sc16．`radio.perf.wire_bytes_per_sample` で足りる |
| G15 | 合う／語彙 | N1 は retune ごとに位相がランダム（`random`），B は資料なし（確かめるまで `random`），H も資料なし．LO の共有は design 33 の `lo_group`（形だけ決めて未実装）．N2・X4 の LO の export／import も同じ形 |
| G16 | 合う | どの装置も資料に値がない．RM-23 の値を bench で測る |
| G17 | 合う | 繰り返しはどれも host から．N1・N2・X4 の Replay は 8c |
| G18 | 語彙 | 今の枠にない安全の設定：antenna port の給電（H は 3.3 V・50 mA，B は方向ごと），amp の on/off，N321 の LO 出力，H Pro の trigger 出力．`RfEnvelope` に optional の欄を足す（schema の再生成．凍結後も追加で足せる） |
| G19 | 語彙 | RM-28 の selector は X3xx の形だけ．機種ごとに layout の文法と profile を足す（H：型番と board id，B：xA4/xA9 と FPGA の版，N1・N2：MCR と image，N321 の LO 分配 board） |
| G20 | 合う | 値はほとんど資料になく bench で測る．timed command のない装置では lead と queue は 0 で，RM-6 の `min_command_lead: None` になる |
| G21 | 合う | どれも呼び出しの戻り値か poll で分かる（H：`hackrf_is_streaming`，B：`BLADERF_ERR_NODEV`，N1・N2：MPM の claim の失効）．`DEVICE_LOST` で表せる |
| G22 | 合う（8b） | N1・N2 は timekeeper 1つ．X420・X440 は2つで lockstep（`set_time_next_pps` を勧める）．TM-18 の「Provider が自分の timekeeper の relation を書く」で表せる．8b で決める |
| G23 | **Kernel の文言と radio の規則（K3）** | X440 は MCR 500 MHz（sample 2 ns）で，command の時刻を 16 ns ごとにしか比べない（UHD v4.10.0.0 の `timed_cmd.dox`，`33-facts-n3xx.md` G20）．UC-6 が許すのは次の sample の瞬間へ進めることだけで，8 sample の格子へ進めることは許していない |

### 5.2 Kernel に関わるもの

**K1 時刻指定で変えられない更新（G4）．** key の update class は Vocabulary の `KeyDecl` だけから来る（RS-52，`design/04-run-and-session.md:261`．validate と admission は同じ `declared_class` を使う，`crates/ezsdr-kernel/src/spec.rs:1204-1228`）．radio の周波数と gain は `hardware_timed`（RM-27）で，UC-6 は「装置が effective instant にちょうど適用する」ことを Provider に課す．`at` のない更新にもこれが当たるので，N1・H・X410・X440 は周波数を設定するだけで UC-6 を破る．design 33 D6 はこれを「これらの装置を実装する phase の前に設計する」と先送りしていた．Kernel が要るのは，**今の key のまま，できる装置では timed の更新を残す**場合だけになる．周波数と gain を全 Provider で `cold` にすれば Vocabulary だけで済むが，UBX・OBX の timed tune（RM-9 の `random_unless_timed_tune`）を今の key で使えなくなる（Sol の S1）．

**K2 時刻を持たない装置と，観測できない故障（G1〜G3，G5〜G7．主に H，G7 は B も）．** 今の Kernel は，時刻を持ち故障を観測できる装置を前提にしている：
- TM-13b：受信の SampleClock の origin は「最初の sample を予定した root tick」．
- KC-15・RM-25：T0 はどの stream の sample の瞬間でもある．SB-16 の schedule は T0 からの stream clock の offset で書く．
- TM-16a・TM-16b：Authority の governed の集合と，「`NotGoverned` は，その domain を進める timekeeper がこの Run にないこと」．
- SC-13：「v4.0 の producer はどれも timestamp を持ち，`lost` を必ず知る」．
- SC-25：underflow は起きた時刻を持つ event．

H は stream を始めるまで数えるものがなく，始まりの時刻は host の時計で推すしかない（遅れは資料にない）．欠落は回数と最長の長さしか分からず，位置も合計も分からない．poll で欠落に気づく前に，欠落より後の sample を続いた index で出してしまう．H の underflow は時刻が分からず，B の underflow は host から観測できない．「数える root を governed にしない」という逃げ方は T0 と SB-16 の問題を解かず，進められている counter を ungoverned と呼ぶことは TM-16b と食い違う（Sol の S3）．**Kernel を変えずに済むかは示せていない．**

**K3 command の格子（G23）．** UC-6 の「格子でしか適用できない装置は次の sample の瞬間へ進める」は，command の格子が radio の tick（MCR）より粗い装置（X440）を表さない．X3xx の command は MCR の tick（200 MHz で 5 ns）で適用され，これは Root の tick で，間引いた stream の sample（1 Msps で 1 µs）とは違う（`design/18-uhd-radio.md:60,203`）．Kernel の型は変わらないが，UC-6 の文言だけでは足りず，radio の実効の瞬間の規則（RM-21）と Provider の実装（`crates/ezsdr-radio-uhd/src/provider/control.rs:561-592` は丸める前の `e` で予約と射影をしている）も変わる（Sol の R1）．

## 6. owner の判断

一件ずつ説明する．Sol のレビュー（2026-10-25）を受けて書き直した．findings の内訳：major 7（S1〜S6，R1），minor 1（S7）．どれも当てはめや推奨の誤りで，Kernel の設計の仕組みに由来する故障ではないので，failure-mechanisms には数えない．

### 判断1（K1）：時刻指定で変えられない更新をどう表すか

二つの案がある．
- **(a) 実効の宣言を instance ごとに持つ（Kernel）．** Provider の instance が，key ごとに `hardware_timed` を `cold` に狭めて宣言できる．宣言は bound な parameter ごとに一つで，validate から Run の終わりまで変わらず，device の node と stream の node のどちらを target にしても同じ class になる（RM-12 は両方を target に許す．Sol の S2）．変わるもの：RS-52，UC-1，SB-16，KC-17，KC-24 の手順5，共有の `declared_class`，`ProviderInstance` とその schema．radio の RM-21・RM-25 はこの実効の class を使う．timed tune の要る実験は，capability `radio.{rx,tx}.c<n>.timed_tune: eq true` で合わない装置を断る．値で可否が変わる装置は，範囲全体で `cold` と宣言する．
- **(b) 全 Provider で `cold` にする（Vocabulary だけ）．** 周波数と gain の key を `cold` にし，timed の更新は，できる装置だけが宣言する別の key に分ける．Kernel は変わらない．一方で，一つの物理量に二つの key ができ，両者の順序（UC-2 は key ごと）と configuration of record の扱いを Vocabulary で決める必要がある．

**推奨：(a)．** key が一つのまま，§59 の「BindingProfile だけを変える」と，今の UBX・OBX の timed tune を保てる．Kernel の変更は凍結前なら小さく，一度で済む．
- 利点：同じ Spec が timed の装置と untimed の装置で同じ key を使う．untimed の retune は `cold`（UC-3）として，stream の切れ目に見える．
- 欠点：Kernel の宣言の道に，上の規則と型が加わる．untimed の装置では retune のたびに受信が途切れる．
- 退けた案：timed の更新をやめて全部 `cold` にし，別の key も作らない（UBX・OBX の位相の揃え方を失う）．新しい class（「host の時刻でおおよそ」）を Kernel に足す（意味の弱い概念を凍結する Kernel に入れる）．機種ごとに Vocabulary を分ける（§59 の比較ができない）．

### 判断2（K2）：時刻を持たない装置（HackRF）を v4.0 で受け入れるか

**推奨：受け入れる．その模型を，凍結前に短い設計メモと amendment で決める．Kernel の変更が要る見込みとして扱う．** メモが決めること（Sol の S3，S4）：
1. 受信の origin と T0 の関係．primary root を何にし，誰が governed にするか（TM-16a・TM-16b）．
2. untimed の装置で許す schedule と `at`（SB-16．`radio.hardware_time: false` の Provider の規則，G5）．
3. 欠落を観測できない間に出した sample の扱い．publish してよい範囲を，気づいていない欠落を含まないように区切るか，最初の欠落で止めるか．
4. 時刻の分からない故障（H の欠落と underflow，B の underflow）．SC-13 と SC-25 の「時刻と数を知る」約束を，その装置にどう当てるか．
- 利点：凍結の前に HackRF の可否と Kernel の変更点が決まる．時刻の約束を弱めるなら，それを一度だけ明示して決められる．
- 欠点：凍結前の設計の作業が1本増え，Kernel の規則が変わる見込みが高い．
- 退けた案：HackRF を v4.0 の範囲から外す（owner 2026-10-24 の順番では HackRF One の Module が最初で，凍結後には Kernel を変えられない）．今ここで模型まで決める（論点が4つあり，メモ1本分の大きさがある）．

### 判断3（D1）：bladeRF の Run の中の rate の変更

**推奨：最初の bladeRF の Module では，rate を binding の時に決める（layout の selector の値．description の rate の格子はその1値）．保守的な制限であって，Kernel の規則から強制される結論ではない（Sol の S5）．** Derived の比は有理数でよい（TM-10）ので，固定の rate の root に写す案は数の上では成り立つ．ただし rate の変更は untimed で（G4），変更の瞬間のカウンタの値が分からず，カウンタが 0 に戻るかも INFERRED なので，正確に写せるかは示せていない．H はカウンタを持たないので，この判断の対象ではない（K2）．
- 利点：Kernel を変えずに最初の Module を出せる．X3xx の `master_clock_hz` と同じ扱いになる．
- 欠点：bladeRF では Run の中で rate を変えられない．
- 後で足せる道：変更の境界でのカウンタの振る舞いを bench で確かめたら，固定の rate の root への写しか，新しい root（TM-16a は governed の集合を宣言するもので，Run の root を固定しない）を検討する．

### 判断4（K3）：command の格子

**推奨：UC-6 の丸めの例外を「装置が格子でしか適用できないときは，その格子の次の瞬間へ進める」に広げ，radio の amendment で次を決める．**
- 格子の宣言：どの clock に対する格子か，その起点（anchor）と，格子に揃えるときの丸め方．capability（例：`radio.timing.command_grid_ns`）で宣言する．一つの instance に一つの格子で全部の command の道を覆えるとは仮定せず，装置ごとに確かめる．
- Provider は予約の前に，装置が実際に適用できる command の瞬間を決める．その瞬間を，順序，queue の数え方，構成の射影，適用時刻の記録のすべてに同じく使う（RM-21．今の UHD Provider は丸める前の `e` を使っている）．

Kernel の型は変わらず，凍結前に UC-6 の文言を一度直す．radio の規則と Provider の実装は X4xx の Module の段で直す．
- 利点：X440 を今の約束のまま正直に表せる．X3xx の格子は MCR の tick（5 ns）で，Root の tick と同じなので，今の丸め（Root の tick への切り上げ）は変わらない．
- 欠点：Kernel の文言のほかに，radio の規則と Provider の実装に手が入る．
- 退けた案：格子に乗らない instant を Provider が断る（Session の `at` は任意の instant なので，普通の更新まで断ることになる）．

### 記録だけで判断の要らないもの

語彙の追加（G10 半二重，G11 共有の拘束，G12 rate の刻み，G13 周波数で変わる gain と段，G18 安全の欄，G19 機種ごとの layout）は，各 Module の段で spec 07 に足す．凍結後も足せる．Module で閉じるもの（G14 の wire 形式）は各 Module の spec に書く．条件つきの「合う」（G1，G2，G6，G9）と B の遅着（G3）は，各 Module の段で確かめる．8b のもの（G9，G22）は 8b で決める．
