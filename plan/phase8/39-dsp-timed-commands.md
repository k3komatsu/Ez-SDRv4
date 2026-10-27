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
