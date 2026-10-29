# 設計メモ 39：DDC/DUC に届く timed command（設計 33 の D9）

> **訂正（設計メモ [42](42-d9-production-fix.md)，2026-10-27）**：§1.4 の「残った設定は後で上書きする」は VERIFIED（`ddc.v`，`dds_timed.v`）．F3 の機構は，in-flight window と release window の差ではなく，DUC が時刻の続く sample でしか取り出さないこと（F3'，`axi_rate_change.v:396-433`）．F5 の答え：UHD は open のたびに全 block を flush して reset するので，Run の終わりの残りは次の Run に持ち越されない（`rfnoc_graph.cpp:714-722`）．F7（DUC の stale header）は新しい（P5 で確かめる）．§3 の LO/DSP の分割は採らない（AUTO/AUTO のまま）．規則は UHD Module（spec 18）に置いた．

| 項目 | 内容 |
|---|---|
| 段階 | Phase 8 段階 2（設計 [33](33-channels-and-streams.md) の D9「流れていない DDC/DUC への timed command」，[00-overview](00-overview.md) の P4 (1)） |
| 決まっていること | D9 の規則 1〜4（owner 受理 2026-10-16）：DDC/DUC に行く timed command は，その instant に chain を packet が通ると計画で分かっているときだけ渡す；そうでないものは記録して次に chain が流れる前に untimed に適用する；渡した後の cut の扱いは spec と FakeDevice の試験で決める；規則は `ezsdr_radio` に 1 か所．メモ 38 §5：`released`／`released_at` を残した |
| ここで決めること | どの command が DDC/DUC に届くか，「通る」の定義，渡した後の cut，規則の置き場（D9 の 4 を改めるか），実機で先に確かめること |
| risky | はい：装置の queue と，fake と実機の境（AGENTS.md §9） |
| 材料 | [research/x300-dsp-timed-commands.md](research/x300-dsp-timed-commands.md)（UHD 4.10 の host と FPGA，2026-10-24） |

## 1. 分かったこと

1. **DDC/DUC に届くのは周波数の DSP の部分だけ**（VERIFIED）．AUTO/AUTO の timed tune は，radio への timed の SPI（LO）と，DDC（RX）か DUC（TX）への timed の周波数の書き込み 1 つになる．gain は radio だけ．`dsp_freq_policy = NONE` なら DDC/DUC に触らない．
2. **radio の timed command は timekeeper で流れる**（VERIFIED）．sample は要らない．ctrl FIFO は 512 word．時刻待ちの command は後ろを止める．RX の stream command も radio に入る（メモ 38 §4 の INFERRED「stream command が同じ queue を使うか」は，radio の queue について「使う」と分かった）．
3. **DDC/DUC の timed の設定は sample の時刻でしか取り出されない**（VERIFIED）．FIFO は 32．sample が来なければ永久に残る．DDC が満杯になると制御の道が詰まり，host は 10 s で `op_timeout`，装置は再起動が要る．
4. **残った設定は後で上書きする**（INFERRED）．untimed の書き込みは FIFO を通らずすぐ当たるが，残った timed の設定は次に sample が通ると同じ register を上書きする．DUC では timed の値が untimed より優先される（VERIFIED）．
5. **Phase 7 の bench の「8 個目で 3 s 待つ」は radio の queue**（INFERRED，research §5）．DDC/DUC の queue の深さは測っていない．

## 2. 今の UHD Module で起きうること

uhd-control は，channel が stream に入っていて stream が halt していなければ live として渡し（`channel_live`），そうでなければ後の segment の channel に渡す（`targets`）．周波数は 1 回の AUTO/AUTO の tune で radio と DDC/DUC の両方に届く．

| # | 系列 | 起きること | 確かさ |
|---|---|---|---|
| F1 | 受信していない channel（開始前，stream の間，halt していないが segment の外）に timed の周波数を繰り返す | DDC に設定が溜まる．32 で制御の道が詰まり，装置の再起動が要る | 1・3 は VERIFIED．今の Module で 32 に届く系列が作れるかは INFERRED（後の segment に渡す道と，stream に入っているが受信していない channel） |
| F2 | A（周波数）を渡した後に，A の instant より前の cut（Stop，cold change）が予約される．次の segment の開始で configure が untimed に今の周波数を書く | 残った A が次の segment の最初の sample で configure の値を上書きする．A の後に別の周波数があれば，その segment は A の周波数のまま | 4 は INFERRED |
| F3 | 送信の burst の途中で timed の周波数を変える | uhd-tx は sample を in-flight window（10 ms）先まで送るが，uhd-control は release window（3 ms）前に渡す．DUC が e の sample をもう処理した後に設定が着くと，設定は次に来る sample（e より数 ms 後）で当たる．切り替わりが遅れる | INFERRED（DUC が sample を受けた時点で処理するか，radio の手前で時刻まで待たせるか） |
| F4 | burst と burst の間に timed の周波数（周波数ホッピング） | DUC の設定は次の burst の最初の sample で当たる．効果は正しい（間は送信していない）．F2 と同じく後の untimed の configure を上書きしうる | 3・4 から INFERRED |
| F5 | Run の終わり（`Provider::stop`）で，e が `S` より後の周波数を渡したまま | 設定が FPGA に残る．次の Run が同じ装置を開いたとき，UHD が block を reset するかは読んでいない | INFERRED |
| F6 | timed の stream command の前に，release window の中の timed の SPI がある | radio の queue で stream command の書き込みが SPI の時刻まで止まる（最大 3 ms）．開始の時刻が近いと遅れうる | 2 から INFERRED |

## 3. 推奨

**先に実機で F2〜F4 を確かめてから，規則を決める．** 1 の事実は資料とソースで足りるが，F2〜F4 の振る舞い（残った設定がいつ当たるか，DUC が sample をいつ処理するか）は FPGA の中の順序で決まり，ソースを読むだけでは言い切れない．いまつながっている X300 #1＋OBX のループバックで，壊さずに確かめられる．

**probe**（`tests/hardware.rs` に `hw_d9_*` を足す．どれも 8 個以下で，FIFO をあふれさせない）：
- **P1（F2）**：受信していない channel に timed の DDC の shift を 3 個（20・40・60 kHz）渡す → untimed で 0 に戻す（その instant の前と後の 2 通り）→ 受信を始め，tone の周波数が最初の何 sample でどの shift か．
- **P2（F3）**：60 ms の burst の途中（e は開始の 20 ms 後）に timed の DUC の shift（50 kHz）．burst を送る前に渡す場合と，e を含む前半を送った後で e の 3 ms 前に渡す場合の 2 通り．ループバックで tone が動く位置を e から測る．
- **P3（F4）**：burst の間に timed の DUC の shift（50 kHz），次の burst の周波数．次の間に timed の shift（80 kHz）の直後に untimed の 0，その次の burst の周波数（残った timed が untimed を上書きするか）．

probe は `crates/ezsdr-radio-uhd/tests/hardware.rs` の `hw_d9_p1_ddc_entries_wait_for_samples`，`hw_d9_p2_duc_entry_after_its_samples`，`hw_d9_p3_duc_entries_between_bursts`（2026-10-24 に書いた．`--features uhd` の clippy は通る．実機ではまだ流していない）．DDC/DUC の shift だけを変える `UhdDevice::tune_dsp`（`rf_freq_policy = NONE`，`dsp_freq_policy = MANUAL`）を使う．流し方は handoff §6 の docker の形で `<test>` に各名前．どれも結果を出力するだけで assert しない（bench-results に記録する）．

**規則の見込み**（probe の結果で決め直す）：
1. gain は今のまま．
2. 周波数は LO の部分（radio，timed，今の instant）と DSP の部分（DDC/DUC）に分けて渡す（`dsp_freq_policy` で 2 回の呼び出しにする）．DSP の部分は，その instant を sample が必ず通る chain にだけ，sample より先に着く時刻に渡す．通らないなら渡さず，次に chain が流れる最初の instant の値として渡す．
3. 渡した後の cut：受信は装置の stop を，渡した DSP の部分の instant の後まで延ばし（cut より後の sample は届けない），設定を残さない．送信は P2 の結果で決める．
4. 試料・行・event に見える振る舞いは変わらない（流れていない間の DSP の値は誰も見ない）．なので MockRadio と語彙は変えず，規則は UHD Module の中に置く．D9 の 4（`ezsdr_radio` に 1 か所）は，見える規則が無いので改める．FakeDevice に DDC/DUC の FIFO（sample で取り出す，32，満杯で止まる）を足し，Run の後に残る設定が 0 であることを試験で固める．

## 4. 採らない案

- **今のまま全部渡し，chain ごとに残っている数を数えて満杯を `COMMAND_QUEUE_FULL` にする**：装置の内部を実験に見せ，Mock にも DDC の流れを持たせることになる．F2 の上書きは直らない．
- **周波数の timed をいつも LO だけにする（`dsp_freq_policy = NONE`）**：LO の分解能の残りが補われず，周波数がずれる．
- **流れていない chain への timed の周波数を拒否する**：burst の間の周波数ホッピングができなくなる．
- **流れ始めのたびに DSP の値を timed で書き直す**：残った設定が最初の数 sample に当たる（1 sample に 1 つずつ取り出される）．queue も余計に使う．
- **probe をせずに規則を決める**：F2〜F4 が INFERRED のまま risky な実装に入る．外れたら設計からやり直し．

## 5. owner に決めてもらうこと

1. 先に P1〜P3 を X300 #1＋OBX で流す（推奨）．壊れない範囲（FIFO 8 個以下）で，所要は準備を入れて半日の見込み．
2. probe の後，§3 の規則で D9 の 4 を改める（規則を UHD Module に置く）．

## 6. 三振（2026-10-29）：装置に残る状態

owner 2026-10-29：「設計のやり直しをしましょう」．D9b（設計メモ [42](42-d9-production-fix.md) の R4・R2・F7）を直す前に，AGENTS.md §6 の三振の問いに答える．

### 6.1 同じ仕組みの不具合

仕組み `mech:device-state-outlives-call`：X3x0 は chain ごとに状態を持ち越す（DDC/DUC の時刻つきの値，DUC が最後に見た header の時刻，閉じていない burst）．UHD Module の送受信の各経路は，呼び出しごとにその状態を初期状態とみなし，burst の始め方・終わり方・時刻の付け方をそれぞれの場所で決めている．

| 件 | 残る状態 | 経路 | 実機 |
|---|---|---|---|
| R2 | 使われなかった DDC の値 | 受信の早い stop（close／reopen を越える） | P4 A/C，UBX と OBX で VERIFIED |
| F7 | DUC の最後の header の時刻 | 前の Run／process の時刻つきの終わり | P5 A，19 ms 早い．UBX と OBX で VERIFIED |
| #70 | 閉じていない burst | 1 sample の burst（`tx.rs:434-437`），失敗した send の後の EOB（`tx.rs:478-486`） | `hw_d70_*`：次の時刻つき burst が約 150 ms 早い．VERIFIED |
| #72（F3′） | DUC の時刻つきの値 | burst の時刻なしの続き（UR-22） | P7，UBX と OBX で VERIFIED（e＋40 ms まで当たらない） |
| F1 | DDC の値が溜まる | 流れていない chain への時刻つきの周波数 | D9a の R1 で直した |

### 6.2 三つの問い

1. **概念を消すか一つにできるか**：できる．送信は「device burst の始まり・続き・終わり」の形を `Tx` の 1 か所の規則にまとめ，形の種類を減らす．
   - **T1**：sample を運ぶ send はすべて，その位置の時刻を付ける（R4）．時刻なしの続きは無くなる（#72，F3′）．
   - **T2**：device burst を閉じるのは，EOB を付けた 2 sample 以上の send だけ．中身の無い EOB や 1 sample の burst は送らない．閉じる sample が足りないときは 0 を足して 2 にする（#70）．
   - **T3**：送信の streamer を開いたら，最初に時刻なしの 0 の 2 sample の burst を送り，残った header を消す（F7）．
   - 受信は一つの不変条件にする．**R2′**：装置に渡した DDC の値は，その chain を止める前に必ず sample で消費させる．つまり装置の stop は，渡した値のうち最も遅い instant より後にする．cut より後の sample は Module が捨てる．
2. **規則が二つの実装に書かれているか**：Radio Model の規則としては書かれていない．これは装置の事実で，MockRadio は TX_UNDERFLOW も出さない（RM-10）．代わりの第二の写しは FakeDevice の DSP の FIFO の model（設計メモ 42 R5）である．それが実機とずれないよう，実機の probe の系列（P4〜P7，D70）を FakeDevice でも流し，同じ判定になることを試験で固める（差分の harness）．
3. **spec の文が複雑すぎるか**：はい．UR-22 は repeat の時刻なしの続き，空の EOB の詰め物，次の安全な開始（`H + 2`）を別々に書いている．T1〜T3 と R2′ に置き換えれば，送信と受信それぞれ 1 つの規則で済む．

### 6.3 見える振る舞いの変化と owner の決め

T1 で，間に合わなかった続きは装置が捨てる（P6 の starvation：TimeError が並び，続きの sample は出ない）．今は時刻なしで遅れて出る．どちらも `radio.TX_UNDERFLOW` を出す．Mock は TX_UNDERFLOW を出さないので，parity は変わらない．

owner の決め（2026-10-29）：**(a)**．underflow の後の burst の残りを
- **(a) 採用**：今の device burst を T2 で閉じ，残りの Kernel burst は次の時刻つきの位置から新しい device burst で続ける．途中の時刻の飛びは `radio.TX_UNDERFLOW` の `cause: starved` と記録の gap で表す（Stream Contract：burst の中の時刻の飛びは埋めない）．
- (b) 採らない：残りを捨て，次の Kernel burst まで送らない．長い repeat が一度の途切れで終わる．

### 6.4 順番

TXS（送信の send の大きさ，進行中）→ D9b（T1〜T3，R2′，FakeDevice の差分の harness）．T1 は TXS の send の大きさと同じ場所を変えるので，TXS の後にする．D9b は risky（試験計画 → Sol → 実装）．

### 6.5 Sol の事前確認と owner の決め（2026-10-29）

Sol が UHD 4.10 と X300 の FPGA の source で確かめた（[reviews/d9b-precheck-sol](reviews/d9b-precheck-sol.md)）：

- radio は送信中，続きの packet の時刻を見ない．時刻を見るのは idle から始めるときだけで，遅れた packet は既定（packet policy）でその packet だけ捨て，TimeError を packet ごとに返す（`radio_tx_core.v:100-136,337-409`）．DUC は最初の時刻から sample を数える（`axi_rate_change.v:396-433`）．VERIFIED．
- underflow の後，radio は次の受け入れられる packet で再開するが，DUC の数える時刻は戻らない．だから host が sample を飛ばすなら，device burst を閉じて新しい時刻つきの burst で始める．owner の (a) はこの形で実現する．
- #70：DUC で最初の sample が EOB を兼ねると，終わりの語数が出ず EOB が失われる（`axi_rate_change.v:225-269,327-384`）．2 sample で避けられる（D70 の対照）．VERIFIED．
- T3 は stale な header の時刻を消すが，DUC の command の FIFO は消さない．host の送る順は data と control の間で保証されないので，T3 の BurstAck を待ってから configure する．
- #71：DUC の FIFO は時刻 32＋2，値 32＋1．満杯でも止まらず，時刻と値の対応が壊れる（`axi_tag_time.v:157-189`，`axi_setting_reg.v:64-87`）．「流れている chain にだけ渡す」だけでは，渡した後で burst が先に終わると値が残る．
- 受信：DDC の値は sample が流れて初めて消費される．X300 は時刻つきの STOP を持たない（`radio_rx_core.v:171-183`）．

owner の決め（2026-10-29）：#71 を D9b に入れる．**R2′ を送受信の両方に広げる**：装置に渡した時刻つきの DSP の値は，その chain を止める・閉じる前に sample で消費させる．送信は device burst を閉じる EOB を渡した最も遅い instant より後に置き，足りない分は 0 で埋める．受信は渡した最も遅い instant 以後の sample が届くまで流し，cut より後の sample は捨ててから stop する．stop と burst の終わりは最大で release window だけ遅れる．

段階：D9b-1a（送信 T1〜T3，FakeDevice の送信の model を Sol の事実に合わせ，P5〜P7・D70 を差分の harness で再現），D9b-1b（送信の R1〈#71〉と R2′，underflow の後の (a)），D9b-2（受信の R2′，P4 の再現）．各段階の後に実機で probe を流し直す．
