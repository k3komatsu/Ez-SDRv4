# Phase 8 設計 33 — channel と stream：混在 board，channel ごとの設定，複数 stream

| 項目 | 内容 |
|---|---|
| 状態 | **第4版（2026-10-17），Gate P の判断はすべて owner 受理**（D1・D9・D13・D16，P6〈D2〉，§3 の用語．[00-overview.md](00-overview.md) §8）．第1版を Sol 6.1 がレビュー（[reviews/plan-sol-review.md](reviews/plan-sol-review.md)，§8），第2版を Sol 6.1 が再レビュー（[reviews/rereview-sol.md](reviews/rereview-sol.md)）し Opus が独立レビュー（[reviews/rereview-opus.md](reviews/rereview-opus.md)，§9），第3版と追加の判断を Sol 6.1 がレビュー（[reviews/v3-sol.md](reviews/v3-sol.md)，条件つきで Gate P へ，P0 なし，P1 4件・P2 3件，§10）．どれも全件を採った．`design/` の規範文は Gate P の後，実装する diff として書く（AGENTS.md §2） |
| 前提（owner の決定，[00-overview.md](00-overview.md) §8） | P1 (c)：channel と stream を分ける．X310 の値は `radio` 語彙に1か所．形は全 USRP を締め出さないように決め，値を入れるのは手元で測れる X3xx の board（UBX，OBX，CBX，Basic/LF）だけ．timekeeper 1つを device 1つとし，1つの Provider instance が複数の device を持てる．10 GbE 2本（XG）も扱う |
| 書き換える文書 | spec 07（§2，§5 の R1・R2，§7 の1・2，RM-2〜RM-9，RM-11〜RM-13，RM-16，RM-19〜RM-21，RM-23，RM-25，RM-26），spec 09（MR-2〜MR-8，MR-18，MR-34 ほか），spec 08（SE-6 の受信 jitter の stream 名），spec 11（channel の番号），spec 18（UR-2，UR-3〈mcr < 2^28 の round trip の議論は X4xx の前に見直す〉，UR-5，UR-9，UR-12，UR-17〜UR-26），spec 16（EA-9，EA-16，EA-17），spec 03（SB-3 の `radio/rx/0` の例，SB-12，SB-23，SB-33），Vision（§8 の例と「方向ごとに1つの key」と `instances: 2`，§13 の grid の文，§26 の「UBX/CBX/SBX は timed tune で位相差が一定」と2値の capability：CBX は UHD で timed の位相同期を持たない〈`sync.dox:165-205`，`db_cbx.cpp` に `sync_phase` なし〉），Kernel は1か所（D2：SB-2 と `KeyDecl`） |
| 言語 | 日本語．`design/` へ移す規範文は英語 |

## 1. 何を解くか

今の `radio` 語彙は，1台の device に「受信の stream 1本」と「送信の stream 1本」しかなく，周波数・利得・antenna を方向ごとに1つしか持てない（RM-3，RM-4，RM-5）．そのため次のことが表せない：

1. **混在 board**：slot A に UBX，slot B の TX 側に LFTX と RX 側に BasicRX（owner が実験でよく使う構成）．
2. **channel ごとの設定**：UBX を 2.4 GHz，BasicRX を 0 Hz（DSP の tune だけ）にする．BasicRX の2入力を antenna `A` と `B` に分けて実数2系統で受ける．
3. **設定の違う channel の aligned stream**：UBX の受信と BasicRX の受信を1本の stream に並べ，同じ block で受ける（owner：必要）．
4. **rate の違う複数の stream**：X3xx は slot ごとに DDC が2つあり，channel ごとに rate を変えられる．1本の stream の中は rate が1つ．
5. **装置の自由度**：200 MHz 固定の master clock，200e6/N（N=1〜512）の格子，方向ごとに1つの throughput の上限，一律の timed tune．調査（[research/usrp-family-limits.md](research/usrp-family-limits.md)）が X1〜X17 として挙げた制約のうち，形を今決めないと後で tree・id・schema を作り直すもの．

## 2. 根拠

- **UHD の構造（v4.10.0.0，[research/x300-uhd-facts.md](research/x300-uhd-facts.md)）**：X300 は slot ごとに radio block 1つ，DDC 1つ（2 port，port ごとに独立の rate），DUC 1つ（1 port）．受信は最大4 channel，送信は最大2 channel．Basic/LF の受信は frontend "0" と "1"，送信は "0" だけ．X300 で Basic/LF の mode（AB/BA/A/B）は antenna で選ぶ．timed command の queue は block ごと（radio，DDC，DUC）．timekeeper は1台に1つ．1本の streamer の中は rate が1つ（multi_usrp が揃える）．daughterboard の clock は方向ごとに両 slot で共有なので，組み合わせによっては開けない．X300 の受信の利得には，slot の2入力に共通の ADC の利得（0〜6 dB）が含まれる（`x300_adc_ctrl.cpp:78-86`，`x300_radio_control.cpp:1718-1739`，Sol が確認）．
- **10 GbE 2本の link の選び方**：UHD は接続（streamer の channel 1つ）ごとに，それまでの**割り当て数**が少ない link を選ぶ．帯域は見ない．数は減らない（close しても戻らない）．multi_usrp からは指定できない（`graph_stream_manager.cpp:188-284`）．
- **DDC の decimation の集合**は `{1} ∪ {2^h · c : 0 ≤ h ≤ NUM_HB, 1 ≤ c ≤ CIC_MAX}` で，`NUM_HB` と `CIC_MAX` は FPGA から読む（`ddc_block_control.cpp`，orchestrator が確認）．X300 の標準 image では 3 と 255（調査役の読み，bench で確かめる）．今の格子は N=257〜511 の奇数を通すが DDC は作れない．
- **UHD の C API**：`uhd_usrp_get_{rx,tx}_rates`，`uhd_usrp_set_{rx,tx}_subdev_spec`，streamer の複数作成はある（`usrp.h:116-193,288-294,494-521,682-709`，Sol が確認）．各方向1 streamer は v4 の `Device` の制約で，UHD の制約ではない．
- **v3**：channel のまとまりごとに rate・freq・gain・ant を設定し，streamer の channel は別に選べた（`v3/cpp/uhd_usrp/multiusrp.cpp:90-217`，`:387-412`）．
- **Kernel**：設定は fragment ごとの平らな表（KC-25，KC-27）．Session の暗黙の Spec は binding ごとに resource 1つ（SB-T2 の3行目）．Action の宛先は resource の下の path（KC-23）．`KeyDecl` は key の完全一致で引き，SB-2 の5つの key の位置と SB-45 は同じ lookup を通る（`registry.rs:194`，`spec.rs:1171-1199`）．key の segment は `[a-z][a-z0-9_]*`（SB-T0）．Provider への値と selector の値は scalar か，scalar の list・map（SB-4，SB-5，KC-24 step 8）で，入れ子は書けない．

## 3. モデル

```text
Provider instance（UHD で1回だけ開く．Phase 8 は1台，8b では複数台）
└── <r>                      radio.device      Spec の resource が binds する node（今と同じ kind）
    │  ports:  rx0, rx1, …    受信 stream ごとに1つ（ezsdr.stream.cf32）
    │  capabilities:         channel ごと・stream ごと（radio.rx.c<n>.*, radio.rx.s<k>.*）と，instance 全体の値
    │  設定（fragment は1つ）：radio.{rx,tx}.c<n>.{frequency_hz, gain_db, antenna, stream}，radio.{rx,tx}.s<k>.sample_rate_hz
    ├── t<i>                 radio.timekeeper  timekeeper 1つ（owner の「device」）．CLOCK_LOST などの source
    ├── rx/c<n>              radio.rx_channel  論理 channel．event の source．capability も設定も持たない
    ├── tx/c<n>              radio.tx_channel
    ├── rx/s<k>              radio.rx_stream   SampleClock の stream id，start_rx・Stop の宛先
    └── tx/s<k>              radio.tx_stream   TxBurst の宛先
```

- **所有と番号の scope**：instance が channel，stream，timekeeper をすべて所有する．channel の番号 `n` と stream の番号 `k` は instance の中で方向ごとの通し番号．timekeeper の番号 `i` も instance の中の通し番号．binding の layout が，論理 channel `n` をどの timekeeper・slot・board・frontend に当てるかを決める（UHD の subdev spec と同じ役目）．
- **channel の番号は実験の論理番号**：Spec は「c1 を 10 MHz に」と論理番号で書き，binding がそれを物理の frontend に当てる．capability の matching は，できない要求（UBX の論理 channel に 0 Hz）を拒否するが，能力の同じ2本の配線の入れ替えは見分けられない．それは約束しない．
- **stream** は同じ rate の channel の集合で，channel の番号の小さい順に block の channel になる．stream の番号 `k` は 0〜（その方向の channel 数 − 1）．node は常にあり，channel が1つも入っていない stream は SampleClock も block も持たない（今の RM-3 の「channel 数 0」と同じ）．
- **aligned と coherent は別**：1本の stream の channel は sample 単位で揃う（aligned）．位相の関係が分かる（Vision §25 の CoherentGroup）のは，全 channel の phase の振る舞いがそれを許すときだけ（D13）．
- **用語（owner に確認する）**：owner の決定の「device」は timekeeper にあたり，tree では `t<i>`（`radio.timekeeper`）．root（kind は今と同じ `radio.device`）は instance 全体を表す．そのため RM-2 の `radio.device` の意味は「device 1台」から「instance」に変わり，timekeeper を2つ持つ instance のうち1つだけを Spec が binds することはできない（instance 全体を binds する）．
- **今作る node と，形だけ決める node**：Phase 8 で読むものがあるのは root と stream の node（SampleClock，宛先，port）．channel の node（`rx/c<n>`）と timekeeper の node（`t<i>`）は id の形だけを決め，読むもの（channel ごとの event，複数 timekeeper）ができたときに作る（足すだけなので後からで締め出さない）．
- **timekeeper が複数のとき**（8b，X420，X440）：stream は timekeeper をまたいでよい（Provider が alignment を宣言するとき）．timekeeper どうしの時刻の関係（root を共有するか）は 8b で決める．Phase 8 で実装するのは timekeeper 1つだけ．

## 4. 決定（推奨と退けた案）

### D1 channel ごとの設定をどこに置くか — **owner 受理（2026-10-16「D1はこれでOK」）**

**推奨：instance の fragment の中に，channel と stream の番号を含む key で置く**（`radio.rx.c1.frequency_hz`，`radio.rx.s0.sample_rate_hz`）．channel と stream は tree の node として持つが，設定と capability は root の key にする．

受理した (c) の文面は「channel の node が設定と capability を持つ」で，この推奨は置き場が違う（意味は同じ：channel ごとの設定，同じ rate の channel の集合としての stream）．

- 利点：1つの所有者（instance）の設定全体が，今の純粋な `coerce`（KC-26）と admission にそのまま渡る．そのため channel 間の拘束（D8）と共有資源の上限（D9）の静的な判定が，1つの関数で一貫して決まる．Spec は今と同じく resource 1つで radio を binds するので，同じ能力の node が2つあるときの matching の曖昧さ（第1回レビューの指摘4）が起きない．Session の暗黙の Spec（resource 1つ）も変わらない．
- 欠点：番号の付いた key が増える（X300 1台で設定 key 20個ほど）．Kernel の key の宣言を1か所変える（D2）．
- 退けた案：
  - **(K2) Kernel の設定を fragment × node path × key にする**：1つの fragment のまま node ごとに設定を持てる（第1版の「必ず fragment をまたぐ」は誤りで，撤回する）．ただし `PrepareReport.effective`，KC-27 の表，`Requested`，SB-30 の検査，Spec の `ResourceReq` の形を広く変える．D1 で得られるもの（所有者全体を1回で判定）は同じなので，変更の小さい方を採る．
  - **(E) channel を Spec の resource にする**：channel ごとに fragment ができ，stream の membership を fragment どうしで結ぶ方法がない．stream の port の置き場もない．
  - **値を list や map にする**：SB-5 と KC-24 step 8 が Provider の値を scalar に限り，capability との比較の意味もない．

### D2 番号の付いた key をどう宣言するか（Kernel の変更1か所）

**推奨：`KeyDecl` に「番号の付く segment」を1つ持たせ，`registry.key_decl` が完全一致の次にこれで引く．** 例：`KeyDecl { key: radio.rx.c.frequency_hz, indexed: Some(2), … }` は，segment 2 が `c` に番号の続く key（`radio.rx.c0.frequency_hz`，`radio.rx.c12.frequency_hz`）をすべて宣言する．決めること：

- `indexed` は 0 始まりの segment の位置で，Vocabulary の prefix の segment は指せない．その segment の宣言は英小文字の語（`c`，`s`），具体の key ではその語に ASCII の10進の番号が続く．番号は先頭の 0 を許さない（`c0` は可，`c01` は不可），9桁まで．
- 1つの具体の key に当たる宣言はちょうど1つ．重なり（完全一致の宣言と番号付きの宣言，番号付きの宣言どうし）は **全 Vocabulary を横断して** 登録で拒否する（今の registry は id の重複しか見ない，`registry.rs:47-72`）．
- 拒否の文面は具体の key を名乗る（`check_constraint_kind`・`check_value_kind` は今 `decl.key` を報告する，`binding.rs:303,321`．番号付きの宣言では番号のない形が出てしまう）．
- 番号の付く segment は1つだけ．GPIO の bank と pin のように2つ要る形は，bank を別の resource にするか，そのときに決める．
- 番号のない形（`radio.rx.c.frequency_hz`）は宣言の書き方で，要求の key としては拒否する．
- `policies.coercion` の key は具体の key の完全一致のまま（番号の範囲への policy は作らない）．
- **存在しない番号の拒否は Provider の責任**：coercible な key は capability がなくても `coerce` に進む（SB-7）ので，`DeviceDescription::coerce` は layout にない channel・stream の番号の key を `RM-4: no such channel` で拒否する．
- テスト：SB-2 の5つの位置（`requires`，`needs[].requires`，output の `params`，`policies.coercion`，schedule の更新）と，`validate()`・`plan()`・`prepare`・Session の admission を通る．`KeyDecl` を含む schema（`vocabulary_descriptor` ほか）を再生成する．

- 利点：Kernel は key の意味を今どおり読まない（OV-21）．GPIO の bank や pin（Vision §39，凍結の後）にも同じ形が使える．凍結（Phase 10 の後）の前に入れられる．
- 退けた案：番号ごとに key を列挙する（上限の決め打ちで，64 channel なら千を超える key）．

### D3 stream の membership と，channel の移動

**推奨：channel ごとの scalar key `radio.{rx,tx}.c<n>.stream`（int．-1 はどの stream にも入らない），update class は `cold`．** 既定は受信の c0 が stream 0，ほかは -1（今の既定「受信 1 channel，送信 0」と同じ）．

channel を stream から stream へ移す（`c1.stream` を 0 から 1 へ）更新は，Provider の中の**1つの移動の操作**として計画する（Kernel の汎用の transaction は作らない）：

1. 移動元と移動先の両方の stream を，同じ effective instant で区切る．その instant は両 stream の先に受けた `cold` 変更のうち遅い方で床を作る（RM-25 の床を2本に広げる）．
2. 両 stream の新しい segment は新しい SampleClock で始まる（channel 数と rate が同じでも作り直す）．origin は RM-25，間は gap にしない．
3. 物理の channel は，移動元の streamer が離してから移動先の streamer が取る（UHD Module は古い streamer を閉じてから新しいものを開く）．
4. どちらかの設定や read-back が拒否されたら，両方の stream が halt する（RM-25 の halt を2本に広げる）．
5. 送信では，移動元と移動先の**両方の** held burst のうち区切り以後のものを取り消す（`COMMAND_REJECTED`．両方とも clock が替わるので，通常の `cold` 変更と同じ）．古い clock の burst を新しい channel の並びで解釈し直すことはしない．
6. 各 segment の channel の並び（論理番号の列）は segment ごとに固定し，Provider の Manifest section に残す（Kernel の `SampleClockRecord` には足さない：足すと2つ目の Kernel の変更になる）．
7. **区切りと準備の時刻**：共通の effective instant から，各 stream の cut をそれぞれの古い clock の sample に丸める（RM-16）．移動先の ready instant には，移動元の streamer の停止と解放，必要な設定の完了を含める（RM-25 の ready の項を2本に広げる．Mock も同じ項を使う）．
8. **止まっている stream（Sol の v3 P1-1）**：3つを分ける．**off**（`Stop` 済み）は membership の更新では再開せず，`start_rx`（受信）か次の burst（送信）を待つ（RM-21）．**on で channel がない**stream に最初の channel が入るのは，channel 数 0 からの enable と同じで，RM-25 のとおり次の segment を始める．**halt**（拒否で止まった）stream は，成功した `cold` 変更（membership の更新を含む）で RM-25 のとおり解除され，on で channel があれば始まる（今の timeline と同じ，`timeline.rs:320-335`）．拒否のときは，両方の stream の記録上の membership，後の計画，再開の条件を揃える．
9. **1か所で実装する**：「同じ床の instant で2本の line に記帳し，拒否なら両方 halt する」は `ezsdr_radio::timeline` の1つの関数にし，両 Provider がそれを呼ぶ（`mech:radio-rule-implemented-twice` が9件，AGENTS.md §6）．実装（段階 1b-2b，2026-10-17）では，移動先の開始が移動元の今の計画に依存し続けるので，関数ではなく instance の全 line を持つ型 `timeline::Lines` にした．移動先の item の `release`（`ready` とは別の field）を `Lines` だけが，移動元の今の計画から（channel を持ち移動より前に始まった segment の終わりの最大）どの操作の後でも計算し直す（spec 07 RM-34）．

`ezsdr_radio::timeline` は stream ごとの channel の集合を持つように広げる（今は channel の数と rate だけ）．

- 退けた案：stream ごとに channel の list を持つ（list は D1 と同じ理由で不可）．bit mask の int（読みにくく，上限が64）．

### D4 stream の node，port，数

**推奨：stream の node は方向ごとに channel 数と同じ数だけ常に宣言する．channel のない stream は node だけあって SampleClock はない．受信 stream `k` の block は root の port `rx<k>` から出る．** 今の port `rx` は `rx0` に改名する．

- 退けた案：stream の数を selector で決める（node の数が binding で変わる）．port を stream node に置く（graph の link は Spec の resource 名の component から出る〈SB-15〉）．

### D5 description と selector

**推奨：`DeviceDescription` を「motherboard の description」と「論理 channel ごとの front end の description の列」から組む．** profile 名は motherboard の系統ごとに1つ（`x3xx`）．MockRadio と UHD Module は同じ selector の書式（`ezsdr_radio::device` が解析）と同じ値を使う．

selector は SB-4 で書ける平らな形にする（入れ子の object は書けない）：

```json
"selector": {
  "id": "radio",
  "master_clock_hz": [200000000],
  "image": ["xg"],
  "links": [2],
  "link_gbps": [10],
  "rx": ["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"],
  "tx": ["0:A:ubx:0", "0:B:lf_tx:0"]
}
```

- `master_clock_hz`・`image`・`links` は timekeeper ごとの list（長さが timekeeper の数）．`rx`・`tx` の要素は `"<timekeeper>:<slot>:<board>:<frontend>"` で，list の位置が論理 channel の番号．UHD Module だけの key（`args`，`clock_source`，`time_source`）と Mock だけの key（`block_len_jitter` ほか）は今どおり．MockRadio の `instances` はなくなる（timekeeper の数が代わる）．
- motherboard の description（`x3xx`）：master clock の許される値，image ごとの DDC/DUC の decimation の集合（`NUM_HB`，`CIC_MAX`），link 1本の bytes/s，TimingEnvelope，startup，queue の単位（D9），board の clock の要求の両立表（D8）．
- front end の description（`ubx`，`obx`，`cbx`，`basic_rx`，`basic_tx`，`lf_rx`，`lf_tx` の各1行）：周波数の範囲と step（LO のない board は DSP の範囲），利得の段（D8：board の RF 利得と slot に共通の ADC の利得を分けて持つ），antenna の一覧，phase の振る舞い，timed tune ができるか，path delay，board の clock の要求．
- rate の格子は `master_clock_hz / N`（N は image の DDC または DUC の decimation の集合）から作る．
- **開くときの照合（UR-5 を広げる）**：UHD Module は layout から subdev spec を作って設定し，front end の名前，master clock，`uhd_usrp_get_{rx,tx}_rates` が返す rate の**集合**（範囲の端だけでなく）を description と比べ，違えば拒否する．DDC の入力 rate が決まる前は UHD が `{1.0}` を返す経路があるので，照合は master clock と subdev spec を設定した後に行う．board の clock がぶつかる layout は，Mock と UHD Module の両方が description の両立表で拒否する（UHD 自身が拒否するのを待たない）．
- **link の速さ**：HG image の port 0 は 1 GbE で，10 GbE の値のままだと Mock が約8倍緩い（以前からある問題）．selector に timekeeper ごとの `link_gbps`（10 か 1）を持たせ，throughput の上限（D9）はそれで決める．`image` と `links` の本数は D9 で読まないので，形だけ決める．
- **同じ装置への2つの binding**：UHD は同じ装置を2回開くと既存のものを返し，subdev spec を黙って上書きする（`multi_usrp_rfnoc.cpp:2902-2908`）．UHD Module は開いた装置の serial を覚え，同じ serial の2つ目を拒否する．
- **開くときの照合**：周波数と利得の範囲も C API で比べられる（`usrp.h:536,624`）．rate の集合は chain がつながる前は `{1.0}` しか返らないかもしれない（INFERRED）ので，段階 1b-4 の最初に FakeDevice と実機（BP0）で確かめる．
- **`ideal`**：motherboard の description を持たない profile として残す．selector の `rx`・`tx` は整数（channel の数，0〜64，既定 1 と 0），各 channel の front end は `ideal`（周波数・利得の grid なし，phase `deterministic`），rate は任意の整数 Hz，throughput は今の値．`x3xx` は selector の `rx`・`tx`（layout）が必須．
- 退けた案：構成ごとに profile 名（P1 で退けた）．board の値を Module ごとに持つ（D3 監査の二重記述が残る）．入れ子の selector（Kernel の `Value` を広げることになる）．

### D6 timed で変えられるか

**推奨：Kernel の update class は今どおり key ごと（`hardware_timed`）のままにする．** channel ごとの capability `radio.{rx,tx}.c<n>.timed_tune`（bool）は形だけ決め，Phase 8 では作らない（Phase 8 の board はすべて timed tune でき，読むものがない）．

- **これは能力の情報で，admission の解決ではない**．`hardware_timed` は `at` のない更新にも当たるので，timed tune のできない装置（X410，X440，USRP2）では普通の周波数の設定も今の規則のままでは動かない．X420 は帯域によって可否が変わる（調査役の読み，一次資料は未照合）．これらの装置を実装する phase の前に，値によって変わる可否と，timed でない更新の扱いを設計する．Kernel の変更（key の update class を node の capability で狭める）はそのときに決める．
- 記録の訂正：第1版の「省略可能な `ParamDecl.update_class` と一緒に凍結レビューで」は誤り．`ParamDecl.update_class` はすでに省略可能（RS-52，`design/04-run-and-session.md:261`）．

### D7 RF の安全範囲（RM-19）を channel ごとに

**推奨：RM-19 の判定を channel ごとにする．** stream に入っている送信 channel `c<n>` ごとに，周波数が帯域の中，利得が `max_gain_db` 以下，`tx_enabled[n]`，antenna が `antenna_ports` の中．stream に入っている受信 channel ごとに，antenna が `antenna_ports` の中（今の受信 antenna の検査を残す）．`tx_enabled` の添字は論理の送信 channel の番号で，stream の中の位置ではない（membership が変わっても詰め直さない）．`RfEnvelope` の文書の形は変えない．

### D8 channel 間の拘束

**推奨：description に拘束を持たせ，`DeviceDescription::coerce` が instance 全体の設定に対して判定する．** 実在が確かめられたものだけを規範にする：

- **利得は board の RF 段だけ（Opus の P0，Sol の R2）**：X300 の受信の利得（UHD の aggregate gain）は RF 段と，slot の2入力に共通の ADC の digital gain（0〜6 dB，0.5 dB step，優先度 0）の合計で，UHD は優先度の高い RF 段から埋める（`gain_group.cpp:98-156`，`x300_adc_ctrl.cpp:78-86`）．`gain_db` の意味は「board の RF 段の利得」とし，範囲は RF 段の範囲（UBX・OBX・CBX は 0〜31.5 dB，Basic/LF は RF 段がないので送受とも `{0}`）．Provider は aggregate gain にその値を設定する．RF 段の格子上の値なら UHD は全量を RF 段に配り，ADC には 0 を書く（各段の setter は呼ばれる）．適用の後に read-back して確かめる．こうすると channel 間の拘束が生まれず，runtime の更新（KC-25，KC-26 は1回に1 key）で同じ slot の2 channel が互いを拒否し合うこともない．ADC の利得を使う key は，要るときに足す．
- **開くときの利得の照合（Sol の v3 P1-3）**：空の gain name の範囲は aggregate（UBX の受信は 0〜37.5 dB，Basic/LF の受信は ADC の 0〜6 dB）なので，D5 の照合は aggregate ではなく **RF 段の名前の範囲**で比べる（C API の gain name ごとの範囲で，`ADC-digital` を除く）．RF 段のない board は公開の範囲 `{0}`．
- **board の clock の要求の両立**：D5．Phase 8 の board（UBX，OBX，CBX，Basic/LF）にぶつかる組はないので，両立表は形だけ決める．
- **UBX・OBX・CBX の全二重**：UHD の資料は，送信を使う間は同じ board の受信 antenna が `RX2` に固定されると書く（UBX・OBX は `dboards.dox:352-354,453`，CBX は `:309-311`，Opus と Sol が確認）．Mock を実機より緩くしないため，この拘束を入れる（同じ board の送信 channel が stream に入っている間，その board の受信 channel の antenna は `RX2` だけ）．
- **Basic/LF の `AB`・`BA` が同じ board のもう一方の channel を使えなくするか**：UHD の source に排他の検査は見つからない（Sol の検索）．規範にしない．bench で確かめる．
- TwinRX の LO と antenna の共有は書式だけ決め，値は入れない．

### D9 共有資源の上限 — **owner 受理（2026-10-16「D9はそれでOK」）**

**throughput**：UHD は link を帯域でなく割り当て数で選び，その数は close しても減らず，multi_usrp からは指定できない．そのため「合計 ≤ 本数 × 1本」と「1 channel ≤ 1本」では，片方の link に重い channel が寄ったときに実機より緩い（Sol の P0：200・25・200・25 Msps を交互に割り当てると片方が 1.6 GB/s）．

**推奨（Phase 8）：link を指定できるまで，方向ごとの全 channel の合計を link 1本の bytes/s 以下にする．** XG を選んでも throughput の上限は1本分のまま．link を channel ごとに指定するには，UHD の RFNoC の C++ API（`rfnoc_graph::connect` の adapter 指定）で streamer を作る必要があり，multi_usrp の C API ではできない．8c で UHD Module に C++ の層が入る（handoff §2）ので，そこで link を channel ごとに割り当て，上限を link ごとの正確な判定にする．

- 欠点：8c までは XG の帯域を使えない（2 channel × 200 Msps は拒否される）．
- 退けた案：
  - **UHD の割り当てを Module の中で再現する**：数は Run の中の open の履歴で決まるので，純粋な `coerce` では判定できない．UHD の内部の振る舞いに依存し，版で変わりうる．
  - **C++ の層を Phase 8 に前倒しする**：multi_usrp を使う今の UHD Module を，X3xx について RFNoC の API で作り直すことになり，Phase 8 の範囲を大きく超える．
  - **測った構成だけを許す**：記録にない構成の扱いが決まらない．

**timed command の queue**：Phase 8 では今の UR-24 と同じく1列の queue とし，その深さは bench で測った最悪値1つ（P3）にする．Mock はその1列で順番どおりの実行（#62）を模擬する．block ごと（radio，DDC，DUC）の queue と，操作ごとにどの queue をいくつ使うか（footprint）は形だけ決め，測った値がそれを要るときに入れる．

**流れていない DDC/DUC への timed command（Opus の P1-2，Sol の v3 P1-4）**：X3x0 の DDC/DUC は流れる packet から時刻を得るので，packet の流れていない chain に timed command を送ると queue が溜まり，あふれると装置の full restart が要る（`usrp_x3x0.dox:861-863`，VERIFIED）．radio block（RF の LO，利得，antenna の register）は timekeeper から直接時刻を得るので，この問題は DSP の tune など **DDC/DUC に行く command** にだけある（INFERRED，FakeDevice の止まる queue で先に固定する）．規則：
  1. DDC/DUC に行く timed command は，**その instant に chain を packet が通ると計画で分かっているときだけ** device に渡す（release する）．受信は stream が run している segment の中，送信は burst が開いていて in-flight window の中にあるとき．
  2. そうでない command（stream に入っていない channel，off・halt の stream，burst の間の送信）は，記録だけして渡さず，次に chain が流れ始める前に，stream を設定し直す一部として untimed に適用する（今の UR-25 の「direction を設定するときは現在の設定を全部 untimed に適用」と同じ）．
  3. 渡した後に cut が予約されたら：release window（3 ms）より先の command は渡していないので取り消せる．渡し済みで cut より後の instant の command は残りうるので，cut を予約するときに，その chain に渡し済みの command の instant が cut より後なら，cut をその instant の後まで延ばすか，cut を受けたことを記録して次の開始時に stream を設定し直す（どちらにするかは spec で FakeDevice の試験とともに決める）．
  4. この規則は `ezsdr_radio` に1か所で書き，両 Provider が使う．測定スイートでこの状態を作ってはいけない．

**runtime**：静的な組み合わせ（throughput，拘束）は `coerce` で判定する．適用時刻に投影した設定での再判定（RM-7 の今の MR-18 の判定と同じ），queue の数え方，release の順番，read-back の拒否の後の扱いは，`ezsdr_radio` に1か所で書き，両 Provider が使う（spec 22 VH-7 の timeline と同じ方針）．

### D10 timekeeper が複数のとき（8b，X420，X440）

**推奨：Phase 8 では timekeeper 1つだけを実装し，形を §3 のとおり決める．** 所有は instance，番号は instance の通し番号，timekeeper は `t<i>` の node．8b で決めること：timekeeper どうしの root の関係，timekeeper をまたぐ stream の alignment の宣言．X440 の dual-rate（timekeeper ごとに master clock が違う）が1つの root に載るかは X440 を使うときに確かめる．「rate の比が有理数なら coherent」とはしない．

### D14 stream の node を宛先にした更新と Stop（Opus の P1-1）

rate の違う stream を持つ resource では，root を宛先にした schedule の項目の SpecTime の clock が決まらない（SB-16：宛先と同じ stream の clock，なければ全 stream が同じ比のときだけ．`pipeline.rs:1010-1036`）．**推奨：Kernel は変えず，radio Provider が `UpdateParameter` と `Stop` を `<r>/rx/s<k>`・`<r>/tx/s<k>` 宛てでも受ける．** 何を変えるかは key が決め，node は SpecTime の clock を選ぶだけ（stream の node 宛ての `Stop` は今の `<device>/rx` と同じ意味）．RM-12，MR-18，UR-24，UR-26 に書く．

### D15 UHD Module の thread と lock（Opus の P1-3）

**推奨：受信 stream ごとに data thread を1本（uhd-rx-s<k>），送信 stream ごとに1本（uhd-tx-s<k>），instance の全 stream の line は1つの lock の下に置く．** D3 の移動はその lock の下で2本の line に記帳し，「移動元が離してから移動先が取る」は ready の項（D3 の7）で順序づける．hot path の handle（RX_OVERFLOW）は stream ごと．今の UR-25 の「device の呼び出しの間は line の lock を持たない」は引き継ぐ．Phase 7 の「1方向 thread 1本」（spec 18 §3）を stream の数に広げる形で，Kernel 側（KG-2 の data thread）は変わらない．

### D16 antenna を Session で変える — **owner 受理（2026-10-17）**（Opus の P1-5）

antenna の key は update class を持たない（RM-21）ので，Session の `set` は KC-24 step 5 で拒否され，Session の暗黙の Spec は `requires` が空．そのため BasicRX の A/B（実数2系統）も，D11 の `rx_channels[n].antenna` も Python からは選べない．EA-16 は今も必ず拒否される setter を公開している（`design/16-easy-api.md:83`）．Basic/LF では antenna が「実数 A/B か複素 AB か」を決めるので，混在構成の主目的に当たる．

**推奨：channel ごとの antenna を `cold` にする．** 変えると，その channel の入っている stream が区切られ，新しい SampleClock で始まる（rate の変更と同じ）．
- 退けた案：front end ごとの既定値（frontend "0" は `A`，"1" は `B`）だけを決める（複素 `AB` は Spec Run でしか使えなくなる）．antenna を `hardware_timed` にする：X300 の antenna の register の書き込みは command time を引き継ぐので timed にはできる（Basic/LF の frontend の接続，UBX の GPIO・CPLD，`x300_radio_control.cpp:142-148,203-206,296-312`，`db_ubx.cpp:842-871`，Sol が確認）が，全 board について切り替えの完了と整定の時刻までは保証できず，Basic/LF では実数か複素かという接続そのものが変わるので，segment の境界として扱う方が正しい（第3版の「timed command でない」は誤りで，訂正した）．
- **antenna の read-back（Sol の v3 P1-2）**：`Applied` に antenna を足し，UHD の getter で読み返して記録する（今は rate・freq・gain だけ，`device.rs:50-59`）．値が違えば，その変更は拒否され RM-25 のとおり stream が halt する．FakeDevice と Mock の fault（#59 の read-back 拒否の fault）にも antenna を含める．UHD の getter は設定した property の読み出しで，物理のスイッチを独立に測るものではない．

### D11 Session と Python

**推奨：** 
- Session の暗黙の Spec は今と同じ（resource 1つ）．`set("radio", "radio.rx.c1.frequency_hz", 10e6)`．`start_rx` と `Stop` は `radio/rx/s<k>` へ，`TxBurst` は `radio/tx/s<k>` へ．
- Python（spec 16）：明示的な `Radio.rx_channels[n]`（`frequency`，`gain`，`antenna`，`stream`）と `Radio.rx_streams[k]`（`sample_rate`，`channels`，`capture`，`request`，`start`，`stop`）を主にする．送信も同じ．`rx_streams[k].capture` は port `rx<k>` を `feed` にする recorder を使う（EA-17 を改める）．
- `Radio.rx`・`Radio.tx`（stream 0 への近道）を残すかは spec 16 の改訂で決める．残すなら，全 channel への設定は複数の Action になり，途中で拒否されると一部だけ変わることと，channel ごとに値が違うときの読み出しの扱いを決める．互換のためだけの近道は要らない（AGENTS.md §6）．

### D12 古い key と名前

**推奨：** `radio.{rx,tx}.channels`・`sample_rate_hz`・`frequency_hz`・`gain_db`・`antenna` を削る（D1 の key に置き換える）．profile `x310-like`・`x310-ubx`・`x310-obx`・`x310-cbx` は `x3xx` に，MockRadio の `ideal` は残す（Vision §13）．Vision の `x310-like` の記述は改める．version は上げない（AGENTS.md §6）．

**key の対応表（Opus の P1-9）**：

| 今の key | 第3版 | 置き場 |
|---|---|---|
| `radio.{rx,tx}.channels` | `radio.{rx,tx}.c<n>.stream`（D3） | channel |
| `radio.{rx,tx}.sample_rate_hz`（設定と capability） | `radio.{rx,tx}.s<k>.sample_rate_hz` | stream |
| `radio.{rx,tx}.frequency_hz`，`gain_db`，`antenna`（設定と capability） | `radio.{rx,tx}.c<n>.frequency_hz`，`gain_db`，`antenna` | channel |
| `radio.{rx,tx}.frequency_step_hz`，`gain_step_db` | `radio.{rx,tx}.c<n>.frequency_step_hz`，`gain_step_db` | channel |
| `radio.phase_behavior_on_retune` | `radio.{rx,tx}.c<n>.phase_behavior_on_retune`（D13） | channel |
| `radio.{rx,tx}.path_delay_samples` | `radio.{rx,tx}.c<n>.path_delay_samples` | channel |
| `radio.rx.coherent` | 削る．stream の位相の関係は membership（Run 中に変わる設定）で決まり，Run 中に変わらない tree（MA-10）の capability にできない．segment ごとに Provider の Manifest section に記録し，Spec は channel ごとの phase の capability で要求する（D13） | — |
| `radio.full_duplex`，`radio.hardware_time`，`radio.tx.repeat_*`，`radio.rx.block_len`，`radio.timing.*`，`radio.perf.*` | そのまま | instance |

既定値（RM-5）は front end ごと：BasicRX・LFRX の周波数の既定は 0 Hz，UBX・OBX は今の 1 GHz，CBX は 2.45 GHz．

### D13 位相の関係：CBX の `random` と LO の共有 — **owner 受理（2026-10-16「D13はそれでOK」）**

CBX は board の種類の表に入る（D5）が，計画は `radio` 語彙の phase `random` を「CBX がない」として範囲外にしている．CBX は研究室にある（owner，2026-10-16）．今の `x310-cbx` は `random_unless_timed_tune` を名乗り，過大な宣言になっている（`crates/ezsdr-radio-uhd/src/profile.rs:76-78`）．

**推奨：phase の値 `random`（timed tune でも位相が揃わない）を `radio` 語彙に足し，CBX をそれで宣言する．** 1本の stream の位相の関係が分かる（coherent）と言えるのは，全 channel が `deterministic` か，全 channel が `random_unless_timed_tune` で timed tune したときだけ．CBX を含む stream は aligned だが coherent ではない．

- 退けた案：CBX を表から外す（研究室にあるのに使えない）．CBX を今の値のまま残す（Mock が実機より強い保証を名乗る）．

**LO の共有（owner の指摘，2026-10-16）**：X420 の HBX（LO の export/import），X3xx の TwinRX（companion）のように，複数の channel が1つの LO を使う構成では，位相の関係は timed tune で揃うより強い：retune の後も channel 間の位相差が保たれる（それでも経路と温度で変わり，定期的な較正は要る）．Vision §25 の CoherentGroup の `basis` はすでに `lo_sharing` を挙げている（`design/vision/05-coherence-calibration-and-runtime-semantics.md`）．

**推奨：位相の関係を3段に分け，stream の位相の関係はその channel から導く．**

| 段 | 意味 | 宣言 |
|---|---|---|
| aligned | sample 単位で揃う．位相差は不定 | 1本の stream の channel はすべてこれ |
| coherent（timed） | timed tune の後，次の untimed retune まで位相差が分かる | 全 channel が `random_unless_timed_tune` で timed tune したとき，または全 channel が `deterministic` |
| shared LO | 同じ LO を使う channel どうしは，retune の後も位相差が保たれる（較正の有効な範囲は下記） | channel ごとの capability `radio.{rx,tx}.c<n>.lo_group`（同じ値の channel が1つの LO を共有しうる）と，共有するかどうかの設定（LO の source・export．board ごとの値は使う board を入れるときに決める） |

- Phase 8 の board（UBX，OBX，CBX，Basic/LF）には LO を共有するものがないので，`lo_group` も共有の設定の key も形だけ決め，Phase 8 では作らない（TwinRX・HBX を入れるときに足す）．
- LO の共有が物理の配線（X420 の LO の SMA ケーブルなど）による場合，それは binding の layout に書く（Spec には書かない）．
- channel ごとの phase の値：UBX・OBX は `random_unless_timed_tune`（`sync_phase`，VERIFIED），CBX は `random`．Basic/LF は LO がなく DSP の tune だけなので `deterministic` と推測するが，**bench で確かめるまでは弱い方の `random_unless_timed_tune` を宣言する**（Mock を実機より強い保証にしない）．bench に DSP の tune と timed restart を含む位相の確認を加える．
- phase の値の「厳しい向き」（P2 の比較）は `random` < `random_unless_timed_tune` < `deterministic`（右ほど強い保証）．
- **能力・選んだ構成・成り立った関係を分ける（Sol の R3）**：`lo_group` は「共有しうる」能力，LO の source・export と layout の配線は選んだ構成，stream の segment ごとに実際に成り立った関係（basis）は Provider が Manifest に記録する．記録では「timed tune で再現できる位相差」（値は分からない）と「較正で分かっている位相差」を分ける．coherent（timed）が成り立つのは timed tune が成功してから，次の untimed retune，ALIGNMENT_ERROR，CLOCK_LOST，halt のどれかまで．混在（UBX と Basic/LF）の stream は，全 channel が timed tune できる値を宣言していれば coherent（timed）になりうる．
- **較正の範囲**：shared LO でも timed tune でも，位相差は LO の周波数・経路・温度で変わり，定期的な較正が要る（UHD `sync.dox:165-177`，`hbx.dox:107-124`）．「較正は一度でよい」とは言わない．較正の記録（Vision §26 の CalibrationArtifact）がいつまで有効かは，その周波数と状態の範囲に限る．
- MockRadio は MR-34 を広げて同じ値を真似る：`random` の channel は timed tune でも位相をランダムにする．
- Spec は位相の関係を channel ごとの capability の条件として要求できる（例：`radio.rx.c0.phase_behavior_on_retune` が `random` でない）．


## 5. 段階

各段階は AGENTS.md §9 の流れ（実装 → セルフレビュー → Opus → Sol → 修正 → gate）で，tree は各段階の後に緑．

| 段階 | 中身 |
|---|---|
| 1a | description を `radio` 語彙に1か所（motherboard と front end の表），profile 名 `x3xx`（EA-9 の既定 profile，fixture，Python，Vision の記述を**同時に**改名），rate の格子の修正と CBX の `random` と MR-34 の広げ（計画の「値は段階4まで変えない」の例外：誤りと分かっている値を測定まで残さない），方向別の start lead．key はまだ方向ごとのまま（layout は1台・方向ごとに stream 1本・同じ種類の board に限る．各方向の slot は UHD の既定の channel の順の prefix，`[]`・`[A]`・`[A,B]` だけ：subdev spec を作るのは 1b-4）．実装（2026-10-17）：CBX は `random` で retune のたびに位相を引き直し，`coherent` は false（D13） |
| 1b-1 | Kernel：`KeyDecl` の番号の付く segment（D2）と schema |
| 1b-2 | 語彙：channel と stream の key・stream の node・port，D3 の移動を含む timeline，D7〜D9，D14 の判定．**新しい tree は別の builder で作り，今の builder と `rm_26_the_description_builds_the_tree` は残す**．レビューの量を抑えるため2つに分けた（2026-10-17）：**1b-2a** は key・layout・新しい description と tree・`coerce`・設定全体の判定 `violations`（D8 の全二重，D9 の throughput）・channel ごとの RM-19（実装：2026-10-17）；**1b-2b** は timeline の channel の集合，D3 の移動（`timeline::Lines`），D14（実装：2026-10-17）．D9 の DDC/DUC への timed command の release の規則は段階 2（#62 と同じ queue の話） |
| 1b-3 | MockRadio：selector の一時的な key（例：`keys: "channel"`．layout は 1a で入るので，layout の有無では切り分けられない．1b-5 で消す）があれば新しい key だけ，なければ今の key だけで動く（**1つの instance が新旧両方の key を受けることはない**：同じ設定が2つの key で `effective` に載るのを避ける）．複数 stream，channel ごとの設定，SimulationChannel の channel 番号．レビューの量を抑えるため2つに分けた（2026-10-17）：**1b-3a** は挙動を変えない組み直し（stream ごとの状態，line を `timeline::Lines` に．テストは変えない），**1b-3b** は selector の一時的な key のモード，複数 stream，channel ごとの設定，D14 の宛先，実行時の `violations`（実装：2026-10-18．移動と `sim.channel` はこの段階では拒否），**1b-3c** は移動（D3）と SimulationChannel の channel 番号（2026-10-18 に 1b-3b から分けた．実装：2026-10-18．始まった segment は全 channel の untimed tune〈MR-34〉） |
| 1b-4 | UHD Module：同じ切り分けで，layout から subdev spec（1a の prefix の制限を外す），stream ごとの streamer と thread（D15），channel ごとの `apply`，開くときの照合（D5）．**1b-4a**（実装 2026-10-18）：device 境界（`Device` trait，`UhdDevice`，`FakeDevice`）に stream の番号，subdev spec，antenna の read-back，rate の集合，serial を足す．空の channel 列は UHD と同じく channel 0（`sanitize_stream_args`）：fake の streamer も channel 0 のものになり，0 channel に切り替える caller（`tx_open(0, &[])`）の扱いが `UhdDevice` と揃う（HEAD の `UhdDevice` は長さ 0 の配列を C に渡していたが，UHD の `sanitize_stream_args` はそれを `{0}` と読む）．fake の timed command の宛先は front end（radio block）で，spec の変更では front end ごと動くか落ちる（channel の番号では引かない）．markup の区切りは ASCII の空白だけで，3 分割以上の失敗の呼び出し名は `uhd_subdev_spec_make`（`subdev_spec.cpp:19-20,58-59`）．`UhdDevice` は `uhd_subdev_spec_make` が失敗したときも handle を解放する（`subdev_spec_c.cpp:31-36`）．`UhdRadio` の挙動は変えない（stream 0，channel は `0..n`；spec は設定しない）．例外は 0 channel の streamer で，HEAD の `UhdDevice` の `tx_send` は 1 本の buffer を（`samples.len() != stream.channels` で）C の前に拒否していたが，UHD の `sanitize_stream_args` はその列を `{0}` と読むので，今は channel 0 の streamer として C に届く（UHD 側の読みは HEAD から変わらない．実機は BP0，pin テストは未了）．規範の文から外した理由の記録：fake の空 spec・重複の拒否が UHD より厳しいのは，RFNoC の `set_rx_subdev_spec` がどちらも検査しないため（`multi_usrp_rfnoc.cpp:2796`）；残った front end の設定が保たれるのは，UHD の channel が radio block のもので channel map がその register に触れないため（INFERRED）；`serial` が受信 channel 0 の `mboard_serial` を読むのは，受信 channel のない装置ではその読みが失敗しうるため（INFERRED）；streamer のない呼び出しで fake が待つのは manual clock の loop が毎回 clock で待つため（plan 30 §4.7）で，`UhdDevice` は即時に返す（修正の回 1：Sol 1〜6，Opus O1〜O3）．修正の回 2（Sol の再レビュー 1〜3）：spec の変更は，その時点までの期限の来た予約を，離れる側の spec のまま先に settle してから置き換える（`set_subdev_spec`）：外れていた front end に期限が来た予約はそこで捨てられ，戻った channel は既定値から始まる（期限の前に戻った場合は，戻った front end の channel にその時刻で効く）．（INFERRED：channel map から外れている間に期限が来た command を UHD の radio block がどう扱うかは未測定．BP0 で確かめる．）streamer のない stream への `rx_packet_samples` の 0，`rx_recv` の 10 ms の待ちと `Failed`，`tx_async` の timeout の待ちは，manual clock の participant のテストで固定した（置き換えた `UhdDevice` は即時のまま）．`set_subdev_spec` の handle の解放は，`src/uhd.rs` の lib のテストで make・set・free をその test binary の中だけで置き換えて数え，成功・解析の失敗・set の失敗の各経路で 1 回であることを確かめた（UHD の `uhd_subdev_spec_free` は解放の後に NULL を入れるので，同じ変数への 2 度目の free は何もしない：`subdev_spec_c.cpp:41`；`uhd_api_a_refused_subdev_spec_is_freed_once` の説明は，実機の libuhd に対して見ているものだけに直した）<br>**1b-4b**（実装 2026-10-19）：`UhdRadio` の内部を stream ごとに組み直した．挙動は不変．<br>・`Streams` は `timeline::Lines` と stream ごとの `StreamState` を持つ．<br>・thread は stream ごとの列にしたが，今は方向ごとに 1 本で，名前は `uhd-rx`・`uhd-tx` のまま．<br>・`Streams::end` は全 line に End を 1 回記帳する．<br>・prune は stream ごと（`Lines::prune_stream`）．move を記帳する Provider は `Lines::prune` で両方の line を畳む．1b-4d で見直す．<br>**1b-4c へ送るもの**（[報告](reviews/1b4b-report.md) §7 の 1〜8 に加えて 9〜11）：<br>(1) stream ごとの thread の名前 `uhd-rx-s<k>`・`uhd-tx-s<k>`，同じ名前の全 thread の join，`rejected` の `thread` の形（UR-16）．<br>(2) D14 の宛先：`booked_stream` は今 stream 0，`Control` の stream ごとの channel，`rx_packet_samples(k)`．<br>(3) `rx/s<k>`・`tx/s<k>` の node，SampleClock の stream id，event の source．<br>(4) 複数 stream での `Streams::end` の検証（VH-8 harness）．<br>(5) `Channels::first(n)` を layout の channel 列（RM-27）に置き換える．<br>(6) `delivered` の床が line と同じ stream のものであることを，2 本目の受信 stream で確かめる．<br>(7) `Lines::prune_stream` と RM-34（1b-4d）．<br>(8) UR-15・UR-17・UR-25・UR-26 の呼び出しの形と番号．<br>(9) Manifest の plan（`Streams::record`）は同じ方向の全 stream の segment を平らに並べ，stream の番号を持たない．stream の識別と，segment ごとの論理 channel 列を記録して検査する（Sol の minor 2）．<br>(10) 拒否が他方の stream を変えないテストを，item のある line で作り直す（Opus の minor）．<br>(11) 行を当て直すときは，`old` の一致に加えて，名前を変えた識別子が `new` に残っていないかも grep する（gate の U41）． |
| 1b-5 | **切り替え**：server（EA-9，port `rx0`），Python（EA-16，EA-17），fixture を新しい key に移し，古い key・port `rx`・今の builder と一時的な key を同じ段階で削る |

## 6. テストの方針

- Kernel（D2）：番号付きの宣言の登録と引き方（`c0`，`c12` は可，`c`，`cx`，`c01`，10桁は不可，重なる宣言は登録で拒否），SB-2 の5つの位置と各段階を通る要求，`policies.coercion` の完全一致．
- 語彙：layout の解析と拒否（未知の board，slot の重複，Basic/LF の送信の frontend "1"，両立しない board の clock），rate の格子（N=257 は格子の値への coercion で，Spec Run の既定の policy `reject` では拒否，N=258 と N=2040 は coercion なしで受理），存在しない channel 番号の拒否，ADC の利得の拘束，throughput（XG でも1本分），RM-19 の channel ごとの判定（受信 antenna を含む），D3 の移動（両 stream の区切り，新しい SampleClock，片方の拒否で両方 halt，held burst の取り消し）．
- MockRadio と UHD Module（FakeDevice）：同じ selector と Spec で，同じ `effective`，同じ stream の SampleClock，同じ拒否（VH-8 の harness を複数 stream へ広げる）．
- 混在構成の end-to-end（FakeDevice）：UBX の受信と BasicRX の受信を1本の stream で受け，block の channel の順と timestamp を確かめる．rate の違う2本の stream．channel の移動．
- 実機：bench の BP4b（[00-overview.md](00-overview.md) §6）と，D8 の未確認の拘束．

## 7. 残る問い

- D9 の3：渡し済みの DDC/DUC の command と，その後に予約された cut の扱い（cut を延ばすか，次の開始で設定し直すか）．FakeDevice の止まる queue の試験とともに spec で決める．
- D9 の queue の footprint（操作ごとにどの block の queue をいくつ使うか）．bench で測る．
- 8b：timekeeper どうしの root の関係，timekeeper をまたぐ stream．

## 8. 第1版の独立レビュー（Sol 6.1）の扱い

全13件を採った（orchestrator が SB-4，RS-52，timeline，UHD の link の選び方の引用を照合）．

| # | 重大度 | 指摘 | 扱い |
|---|---|---|---|
| 1 | P0 | D9 の「合計 ≤ 本数 × 1本」は link の偏りで実機より緩い | D9 を「合計 ≤ 1本」に改め，link ごとの判定は 8c．owner の判断待ち |
| 2 | P1 | membership の `cold` だけでは2本の stream を切り替えられない | D3 に移動の操作（6項目）を追加 |
| 3 | P1 | selector の例が SB-4 の `Value` で書けない | D5 を平らな形に |
| 4 | P1 | timekeeper＝device と tree が一致しない | §3 に `t<i>` の node と所有・番号の scope |
| 5 | P1 | D1 は (c) の文面と違う．K2 を退けた理由が誤り | D1 に差を明記し owner に確認，理由を訂正 |
| 6 | P1 | D2 の成立条件が広い | D2 に6項目，テストを §6 に |
| 7 | P1 | D8・D9 は runtime まで `coerce` だけでは完結しない | D9 に runtime と queue の footprint |
| 8 | P1 | ADC の利得の共有，clock の競合の Mock 側，AB/BA の排他は未確認 | D8 を書き直し，未確認のものは規範にしない |
| 9 | P1 | aligned と coherent の区別，CBX の範囲の矛盾 | §3 と D13 |
| 10 | P1 | D7 が受信 antenna の検査を落とした | D7 に戻した |
| 11 | P1 | D6 の bool は admission の解決ではない．記録の誤り | D6 を書き直し |
| 12 | P1 | 段階が各段階で緑を保証しない | §5 を並置 → 切り替えに |
| 13 | P1 | 測定の適用範囲と bench の表 | 計画（00-overview）の P2 の記録と §6 を直す |

## 9. 第2版の再レビュー（Sol 6.1）と独立レビュー（Opus）の扱い

orchestrator が引用を照合した（UR-3 の `mcr < 2^28` は spec 18 の `design/18-uhd-radio.md:106` にある：00-overview の記録の誤りを直した）．

| 出どころ | # | 重大度 | 指摘 | 扱い |
|---|---|---|---|---|
| Sol | R1 | P1 | D3 の移動で，移動先が動ける時刻と止まっている stream の扱いがない | 採用：D3 の7〜8 |
| Sol | R2 | P1 | ADC の利得の分け方が決まっていない | Opus の P0-1 と合わせて採用：D8（利得は RF 段だけ） |
| Sol | R3 | P1 | LO の「共有しうる」と「共有している」，coherence の成立と失効，「較正は一度」 | 採用：D13 |
| Sol | R4 | P1 | 新旧の key を並べる期間の規則がない | 採用：§5（instance ごとにどちらか一方） |
| Opus | P0-1 | P0 | ADC の利得の拘束で，Basic/LF の利得が runtime に変えられなくなる | 採用：D8 |
| Opus | P1-1 | P1 | rate の違う stream で root 宛ての schedule が arm で拒否される | 採用：D14 |
| Opus | P1-2 | P1 | 流れていない DDC/DUC への timed command で装置が止まる | 採用：D9 |
| Opus | P1-3 | P1 | thread と lock の形 | 採用：D15 |
| Opus | P1-4 | P1 | D3 の移動を1か所で実装 | 採用：D3 の9 |
| Opus | P1-5 | P1 | Session で antenna を変えられない | D16 として owner に諮る |
| Opus | P1-6 | P1 | `ideal` の形 | 採用：D5 |
| Opus | P1-7 | P1 | 段階 1b-2〜1b-5 で緑を保てない | 採用：§5 |
| Opus | P1-8 | P1 | 書き換える文書の一覧が足りない | 採用：表の「書き換える文書」 |
| Opus | P1-9 | P1 | key の対応表 | 採用：D12 の表 |
| Opus | P1-10 | P1 | D2 を Gate P の独立した判断に | 採用：00-overview §4 の P6 |
| Opus | P1-11 | P1 | raw の測定値を profile に使わない | 採用：00-overview の P2・P3 |
| Opus | P2-1 | P2 | KC-15 の T0 が互いに素に近い decimation で大きく遅れる | 記録．段階 1b-2 で `coerce` が T0 の格子の周期に上限（例：1 s）を置くかを決める |
| Opus | P2-2 | P2 | D2 の細部 | 採用：D2 |
| Opus | P2-3 | P2 | 読むもののない形は実装しない | 採用：§3，D5，D6，D8，D9，D13 で「形だけ」 |
| Opus | P2-4 | P2 | 移動先の held burst，Manifest の置き場 | 採用：D3 の5・6 |
| Opus | P2-5 | P2 | 混在の coherence，Basic/LF の値，phase の向き | 採用：D13 |
| Opus | P2-6 | P2 | UBX・OBX の全二重で受信 antenna が RX2 | 採用：D8 |
| Opus | P2-7 | P2 | HG の port 0 は 1 GbE | 採用：D5 の `link_gbps` |
| Opus | P2-8 | P2 | 同じ装置への2つの binding | 採用：D5（serial で拒否） |
| Opus | P2-9 | P2 | 開くときの照合 | 採用：D5 |
| Opus | P2-10 | P2 | 記録の誤り | 採用：00-overview と §5 |
| Opus | P2-11 | P2 | 計画の漏れ（NB-5，UR-17，BP0・BP5 の順，出口条件） | 採用：00-overview |
| Opus | P2-12 | P2 | 用語のずれ（root が instance，timekeeper が `t<i>`） | §3 に書き，Gate P で owner に確認 |

## 10. 第3版と追加の判断のレビュー（Sol 6.1）の扱い

結論は「条件つきで Gate P へ」，P0 なし．追加の3判断（P6・D16・用語）にはすべて賛成．全件を採った（orchestrator が UHD の引用と `timeline.rs`，`device.rs` を照合）．

| # | 重大度 | 指摘 | 扱い |
|---|---|---|---|
| P1-1 | P1 | D3 の halt と空の stream の復帰が RM-25・timeline と食い違う | D3 の8を off・空・halt に分けて書き直した |
| P1-2 | P1 | D16 に antenna の read-back と不一致の扱いがない | D16 に追加（`Applied` に antenna，不一致は halt，#59 の fault に含める） |
| P1-3 | P1 | 開くときの利得の照合が aggregate だと正しい装置を拒否する | D8 に「RF 段の名前の範囲で比べる」 |
| P1-4 | P1 | D9 が送信の idle と渡し済みの command を覆っていない | D9 を4項目に書き直した．3の細部は §7 に残し spec で決める |
| P2-1 | P2 | 「antenna は timed にできない」は原典と違う | D16 の理由を訂正（`cold` は維持） |
| P2-2 | P2 | CBX にも全二重で RX2 の記述がある | D8 で CBX も同じ扱いに |
| P2-3 | P2 | 古い未決の記述が残る | 状態の行と §7 を同期 |
| 補足 | — | ADC には 0 が書かれる（「触れない」は不正確），R3 の記録の区別，D15 は UR-25 の lock の規則を引き継ぐ | D8，D13，D15 に反映 |

