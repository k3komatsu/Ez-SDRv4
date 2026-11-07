# 設計メモ 47：流れている stream を変えない（帳簿 46 の決定を spec に入れる）

Status: 第 3 版（2026-11-04，orchestrator Opus）．Sol の下書きのレビュー（12 件）と第 2 版の再レビュー（5 件）を入れた（§8）．**owner の決定**：§6 の O1〜O3 はすべて決まった（2026-11-04）．

| 項目 | 内容 |
|---|---|
| きっかけ | owner が帳簿 [46](46-streaming-feature-ledger.md) の F1〜F9 をすべて決めた（2026-11-04，46 §8）．報告 E4 §6 の 3 の順番（owner 2026-11-04，handoff §2）の 1 番目 |
| 決めること | 46 §8 の決定を，Radio Vocabulary（spec 07），MockRadio（spec 09），UHD Module（spec 18），Easy API（spec 16）の規則に落とす．形は案 (A)（owner 2026-11-04）：値の変更は `UpdateParameter` のまま運び，Easy API が開始の Action の前に出す．Kernel の Action の形と admission は変えない |
| 変えないもの | Kernel の Action の形と admission の順（RS-17，RS-19，RS-49，RS-52，`crates/ezsdr-kernel/src/coordinator/admission.rs:35-286`），Stream Contract，Vision §27 の class の集合，§58，§61，§65 |
| 変えるもの（Kernel） | Session の入口と暗黙の Spec の作り方だけ（O1：初期の設定の要求を hash と validate の前に resource の `requires` に入れる．RS-12，SB-22c・SB-T2，KC-1．§2.4）．radio の知識は Kernel に入れない |
| 変えるもの（Vision） | §3 と §66 の Easy API の例（connect の後に `.stream = 0` とする所．§2.5） |
| risky | はい（寿命，順序，fake と実機の境に触れる．AGENTS.md §9） |

## 1. 決定の要約（46 §8 と owner 2026-11-04）

- **channel の組は Run の間変えない**（F4a (d)）．channel の move と空の stream を開くこともなくなる（F5）．
- **受信の周波数，gain，sample rate，antenna は，止めた受信の次の開始（`start_rx`）の前にだけ変わる**（F1 (e)，F4b 改め，F9）．
- **送信の周波数，gain，sample rate，antenna は，空の送信の stream にだけ変えられ，次の burst から効く**（F2 ①，F9）．その burst の booking で新しい送信の clock を登録し，origin はその burst の開始．antenna は将来の board のための予約．
- **Stop と `start_rx` は残す**（F6 (a)）．
- **流れている burst や予約済みの burst に重なる予約は拒む．ちょうど続く burst は受け付ける．repeat 専用に「次の周期の始めで差し替える」**（F7）．同じ長さなら周期はロックしたまま．
- **送信の recovery は今のまま**（F8 (a)）．
- **rate の変更は，開いたままの streamer での実機の確認が済むまで受け付けない**（46 §8 の条件．§4 の各段階の受け入れの条件にする，Sol 12）．
- 形は (A)：値の変更は `UpdateParameter`（class `cold`）．Easy API の `rx.start(at, frequency=…)` などは，値の `UpdateParameter` をすべて出してから開始の Action を出す．**一つの all-or-nothing の操作にはならない**（Sol 7）．部分的な失敗の扱いは §2.4．

## 2. 新しい規則（spec ごと．文は実装の diff で書く）

### 2.1 spec 07（Radio Vocabulary）

1. **RM-27 の class**：`frequency_hz`・`gain_db` は `hardware_timed` から `cold` へ．`antenna`・`sample_rate_hz` は `cold` のまま．`stream` は `cold` から `—`（Run の間は変わらない）へ．channel の組は `prepare` の前の設定で決まる（§2.4，§6 の O1）．
2. **「流れていない」と staging**（RM-21 を改める．Sol 4，5，7）：
   - `cold` change は，その stream が**流れていない**時にだけ受け付ける．流れている時は `radio.COMMAND_REJECTED`（理由つき）で，Provider は値を使わない（Kernel の admitted configuration には残る，KC-27．§2.4）．
   - **受信が流れていない**：`Stop` の後で，**まだ `start_rx` が予約されていない**時（Run の始めの前も）．`start_rx` が予約された後の `cold` change は，その開始が効いて次の `Stop` が来るまで拒む（予約の時に開始の設定の snapshot を取るので，Sol 4 の系列：予約の後の値が開始に入らない・開始の処理中に設定が変わる，を防ぐ）．
   - **送信が流れていない**（受け付けの条件）：開いている device burst，予約済みの burst，owner へ移している途中の burst，保留中の差し替えや設定が，どれもない時．
   - 受け付けた値は**staging**に入る．stream ごとに，次の開始（受信は `start_rx`，送信は次の burst）の設定の snapshot に固定される．`Stop`，loss，`Provider::stop` は staging を消さない（値はその stream の configuration of record として残り，次の開始に使われる）．
   - 同じ stream の staging への値は，届いた順に上書きする．開始の Action はそれより後に届くので，開始は届く前のすべての値を使う．
3. **送信の設定の適用**（Sol 3，5）：
   - 設定は，旧 device burst が閉じ（EOB と padding を含む），装置がその sample を出し終えたと見なせる時（前の burst の終わりの時刻を装置の時刻が過ぎた時）より後に装置に入れる．recovery（F8）とは排他にする．**この安全の条件と余裕は INFERRED で，実機で確かめる**（§4 の受け入れの条件）．
   - 設定を伴う最初の burst の booking で新しい送信の clock を登録する（RM-25，KC-21a）．origin はその burst に決まった開始．開始は，前の burst の終わり + 送信の start lead，設定の完了からの readiness，今の safe front のどれよりも前にならない．
   - Kernel がすでに旧 clock の上に解いた `at`（RS-19，admission の変換）を新 clock に移す手順：root の時刻を保ち，新 clock の格子へ切り上げる（RM-25 の lattice）．
   - RM-31（duplex の board の antenna）と RM-32（throughput）は，staging の値を含む次の configuration で確かめる．X310 の送信の antenna は今は一つの値しか許さない（`coerce` が確かめる）．
4. **`hardware_timed` の radio の key はなくなる**（Q3，Sol）．`radio.timing.command_queue_depth`，時刻つきの値のための `release_window_ns` の使い方，`timeline::timed_instant`，`timeline::covering`，radio の更新の `COMMAND_QUEUE_FULL` とその payload と schema は消す．**残す**：`min_timed_command_lead_ns`（`ProviderInstance.min_command_lead`），startup latency，受信・送信の start lead，送信の in-flight window，overflow restart gap．受信の後続の readiness（RM-25，`timeline.rs:1020-1038`）からは release window の項だけを外し，古い rate での受信の呼び出し一回，delivery allowance，start lead は残す．RM-6，RM-20，RM-35，profile，schema，測定の fixture を同時に変える．
5. **RM-25**：`cold` change は流れている stream を切らないので，「`cold` change が segment を終えて次を始める」「instance 全体の `cold` の floor」は消える．segment を始めるのは T0 と `start_rx`．refusal（開始の設定を装置が拒む）で stream が止まる規則は残る．
6. **RM-34（channel の move）**：withdrawn（一行の stub）．RM-36（`covering`）も呼び手がなくなれば withdrawn．
7. **RM-13〜RM-15（送信の burst）**：
   - **重なりの判断は区間で行う**（Sol 8）：開いている burst，owner へ移している途中の burst，予約済みの burst のそれぞれの区間 `[start, end)` と，新しい burst の区間が重なれば `radio.COMMAND_REJECTED`．repeat の区間は終わりがない（差し替えだけが例外）．有限の burst の終わり（exclusive end）にちょうど続く burst は受け付け，一つの device burst として続ける．
   - **LatePolicy で開始が動いた後も同じ判断をする**（`SendAsap` の移動先が別の予約と重なる場合は拒む）．
   - 隙間のある後の burst は今のまま．
   - **repeat の差し替え**（Q1）：`TxBurst { repeat: true }` に Vocabulary の metadata の印 `radio.repeat_operation = "switch"` を付けたもの（RM-13 の「metadata は空」を「空か，この印だけ」に改める）．Session の verb は増やさず，`start_repeat` に印を付ける（owner 2026-11-07，段階 5：Kernel の compile 規則は metadata を持たない）．印のない repeat は通常の repeat（重なれば拒む）．
     - 流れている repeat に対してだけ．Provider は，Kernel が入れた `at`（RS-19）を下限として扱い，受け付けた時の「今 + in-flight window + device lead」以後で最初の，その repeat の周期の境目（origin + k·N）を選ぶ．
     - **境目の選び方と，装置に渡した sample の境（handed cursor）を進めることを，送信の owner の中で直列にする**（Sol 9）．選んだ境目は後で動かさない．
     - 同じ repeat への保留中の差し替えは一つだけ（二つ目は拒む）．
     - `Stop`，loss，`Provider::stop` は保留中の差し替えを取り消す．F8 で前の repeat が打ち切られたら，保留中の差し替えは取り消す（続ける相手がない）．
     - 新しい波形の長さは問わない．**同じ長さなら周期はロックしたまま**（境目は origin + k·N に一致し続ける）．
     - 選んだ境目の記録（Sol 2）：`{ action_id, predecessor, selected_at, old_period, new_period, outcome }` を Provider の記録の section に書き，action log の `dispatched` の id と結ぶ．Kernel の `LogEntry` の coercion には入らない（RS-19 の coercion は admission の時に作られる）．
8. **RM-16（Stop）**：変えない．「`cold` change が予約済みの burst を取り消す」と，時刻つきの命令の清算（hand-over と `S`）は消える．

### 2.2 spec 09（MockRadio）

- MR-18：流れている stream への `cold` change を拒む．staging と開始の snapshot．時刻つきの更新の queue と floor をやめる．
- MR-16・MR-17・MR-32：重なりの区間の判断，repeat の差し替えと印．
- MR-40（move）は withdrawn．MR-38・MR-39 の move の文を改める．
- MR-33〜MR-35（channel mode の RF の模型）：値の履歴は残す（過去の sample を正しく計算するため）．retune は開始の前の設定だけになる．`random_unless_timed_tune` の board では，開始ごとの設定で位相を引き直す．同じ値の設定，失敗，取り消しで何回引くかを決める（Q4）．

### 2.3 spec 18（UHD Module）

消すもの・改めるもの・残すものの一覧は §5（Sol の洗い出し）．要点：

- **消える**：UR-24 の時刻つきの値の release（保持の列，release window，eligibility，debt，drain，flush，`left`），UR-26 の restore と時刻つきの命令の清算，UR-25 の `Release` と `FromNone` と move の待ち合わせ，from none の開き方，`cold` change が segment を切る道，UR-21・UR-23 の一般の割り込み．
- **残る**：受信の cut と `start_rx` の segment，開始の前の設定と refusal の halt，`RxChain` の Off／Armed／Streaming／Uncertain／Lost，`claim_owner`・`begin_read`・`give_back`（F8，report，shutdown が使う），送信の device burst の続きと最後の sample の取り置き，safe front，T1〜T3，F8，generation．
- **足す**：「流れていない」の判断と staging，開始の snapshot，送信の設定の適用の条件（§2.1 の 3），区間の重なりの判断，repeat の差し替え．
- uhd-control は Action の booking，予約，reference の監視になる（時刻つきの release がなくなる）．

### 2.4 spec 16（Easy API）

- **channel の組は `connect` の時に決める**（§6 の O1）．`Radio.rx_channels[n].stream`，`tx_channels[n].stream` は Run の中では読むだけ．
- **`connect` の初期の要求の契約**（再レビュー 1，2）：
  - Kernel の Session の入口は，radio に依らない初期の要求（resource ごとの key と値の組）を受け取り，**hash と validate の前に**その resource の `requires` に `Constraint::Eq` として入れ，要る Vocabulary の major も暗黙の Spec の `requirements` に宣言する．radio の key を具体的な形に展開するのは Python の client（`radio.<d>.c<n>.stream = Eq(k)` など）．server の connect の frame と schema（EA-6，EA-10）に欄を足す．
  - `tx={k: [channels…]}`・`rx={…}` の意味：**書いた方向の map はその方向の組の全部**（書いていない channel は none）．**書かなかった方向と空の map は，その方向の全 channel が none**（RM-30 の既定は受信・送信ともどの stream にもない，O1 の改め）．同じ channel を二つの stream に書く，存在しない channel や stream の番号，型の違いは，connect の前に client が拒む（黙って後の値を使ったり既定に戻したりしない）．sample の並びは RM-3 の logical channel の番号順のまま．radio が二つ以上ある時は，resource の名で宛先を決める．
  - 初期の周波数・gain・rate・antenna も同じ道で渡せる（`requires` の `Eq`）．既定の BindingProfile の selector（物理の layout）には書かない．
  - **Python の形（owner 2026-11-04）**：stream ごとにまとめる．`ezsdr.connect(rx={0: ezsdr.Stream([0, 1], sample_rate=20e6, frequency={0: 2.45e9, 1: 2.46e9}, gain={0: 10})}, tx={0: [0]})`．値がなければ channel の list だけでよい．`Stream` の値の書き方は `start()` と同じ（channel の番号の dict と stream の rate，O3）．v3 の設定ファイルも一つのもの（channel のまとまり）の値を一つのオブジェクトにまとめていた（`v3/config_examples/x310_UBX_TXRX_v3.json`）．退けた案：値の種類ごとに方向で分けた dict（`frequency={"rx": {…}}` など．一つの stream の設定が散らばる）．
  - **名前**（owner 2026-11-04）：
    - **stream の名前は `connect` で付ける**（Easy API の中だけ）：`rx={"main": ezsdr.Stream([...])}` の key は番号（`int`）か名前（`str`）．名前で `sdr.rx_streams["main"]` のように指せる．番号は server が決める．段階 1．
    - **channel の名前は BindingProfile の radio の selector に付ける**（段階 1b）：並びの要素を `"0:A:ubx:0#loop-rx"` とし，`#` の後ろが名前（RM-28 を改める）．名前は配線の事実（どれとどれがつながっているか，どの antenna か）で付ける．名前のない要素は今の文字列のまま（番号でだけ呼べる）．
    - 規則：名前は profile 全体で一意（radio をまたいでも重ならない．名前だけでどの radio のどの channel かが決まる）．使えない文字は `:`，`#`，空白．数字だけの名前は禁じる．Easy API のどこでも `int` は番号，`str` は名前で，番号と名前を混ぜて書ける．一つの stream に別々の radio の channel が混ざれば拒む．selector の文字列を比べたり読んだりする所は，すべて `#` 以降を外して扱う（MockRadio，UHD Module，profile の照合，parity，試験の helper）．
    - 解決：`connect` の引数の名前は server が profile の selector を見て番号に直す（layout にない番号の拒否と同じ場所）．`connect` の後の `start()` などは，client が `connect` の返事で受け取った名前の表で直す．Kernel，Action，key，action log，記録の識別子は番号のまま（名前と番号の対応は記録された profile から読める）．
    - 利点：プログラムを配線から切り離せる（同じ Python のプログラムを，同じ名前を付けた別の bench の profile や MockRadio の既定の profile でそのまま走らせられる．Vision §58 の 10・14 の考え方を channel の単位で）．複数の USRP の構成で取り違えにくい．退けた案：名前を Kernel の BindingProfile の欄（`aliases`）に置く（Kernel の schema が変わる），selector の要素をオブジェクトにする（要素が文字列とオブジェクトの二通りになる），装置の場所を表す名前（番号の言い換えで，持ち運べる利点がない）．
  - **段階 1 の細かい裁定**（orchestrator 2026-11-04，テスト計画の問い）：stream の名前は一回の `connect` の中で受信と送信をまたいで一意．受信の stream がない radio での `capture` などは EA-16 の別名の規則と同じく `Error`（`connect(rx=...)` を案内する）．番号で書く `rx`/`tx` は一台の radio を設定する（`radio=`，既定 `"radio"`．複数の radio をまとめて書くのは channel の名前〈段階 1b〉か 8b）．`connect` の返事に名前と番号の対応 `Connected.streams` を足す（reply の schema の変更．O2 の返事の変更と同じ段階で）．
  - **Kernel の入口**（orchestrator）：`coordinator::connect` を一つのまま，初期の要求の引数を足す（呼び出し元を直す．入口を二つ残さない）．**layout にない番号の拒否と，書かなかった channel を none にする展開**は，client が layout の channel の数を知らないので，server が Module を組み立てた後，Run を作る前に行う．型や重なりのように client だけで判断できるものは client が先に拒む．
- **開始と設定**（Q5）：`Radio.rx_streams[k].start(at=None, *, frequency=None, gain=None, sample_rate=None, antenna=None)`．`frequency`・`gain`・`antenna` は logical channel の番号を key とする dict，`sample_rate` は stream の値．書かなかった channel は変えない．送信の `repeat(x, at=None, *, …)`・`send(…)` も同じ形．
- **送る前に全部を確かめる**（Sol 7）：client は，channel の番号（その stream の channel か），値の型，波形を送る前に確かめる．
- **送る順と部分的な失敗**（再レビュー 3）：値の `UpdateParameter` を一つずつ出す．各 submit の後，**Kernel の拒否，またはその Action の id を持つ Provider の booking の拒否**（O2）があれば，例外を出し，残りの値と開始の Action を出さない．submit が返るまでに分かるのは booking までの拒否で，後の configure の失敗や LatePolicy の移動の後の拒否は，非同期の event として別に扱う．待つ間に Run が終わった（終わり，freeze，timeout で完了を待たずに返った，`crates/ezsdr-kernel/src/coordinator/paced.rs:185-202`）時は，拒否の event がないことを成功と見なさず，例外にする．
- **読み返す値は admitted の値**（再レビュー 4）：属性と `effective` が見せるのは Kernel の admitted configuration で，Provider の staging や装置に入った値ではない（KC-27）．Provider が拒んだ値も admitted には残る（今の KC-27 と同じ）．そのため，流れている stream に値を出して拒まれた後は，読み返す値と次の開始に使われる値が違いうる．この違いは文書に書く（Provider の staging を読む道は作らない）．受け付けられた値は staging に残り，次の開始に使われる．
- **Provider の拒否の扱い**（Sol 6，§6 の O2）：Kernel の admission を通った後で Provider が拒む場合（流れている stream への change，重なる予約）は，今の client では例外にならない（`python/ezsdr/session.py:110-113` は Kernel の outcome しか見ない）．
- 流れている stream の `frequency` などを設定すると，Provider が拒む（O2 の決め方による）．
- repeat の差し替え：新しいメソッド（名前は設計で決める）．

### 2.5 Vision

- 変えないもの：§27 の class の集合，§58 の 9 と 16，§61 の四つの振る舞い，§65 の不変条件（Sol 11 が確かめた）．
- **変えるもの**（Sol 11）：§3 と §66 の Easy API の例（`design/vision/01-purpose-and-core-boundary.md:53-72,110`，`design/vision/11-boundaries-invariants-and-roadmap.md:167-171`）を，`connect` の時に組を決める形に．§3 の「Easy API の一つの呼び出しは log の一つの entry」（`01-…md:106`）を，(A) の「一つの呼び出しが複数の entry になりうる」に合わせる．
- §58 の 16 の試験：止めた（空の）stream に envelope の外の値を出し，admission で拒まれ，log に残り，dispatch されないことを確かめる．別の試験で，許される値を流れている stream に出して Provider が拒み，action id で対応づくことを確かめる（Q6）．

## 3. 答えた問い（Sol の独立の答えを採った）

- **Q1 repeat の差し替えの形**：`TxBurst { repeat: true }` と metadata の印（§2.1 の 7）．Kernel の Action も admission も変えない．Command に波形を載せる案は，input store と波形の確かめを使い直せないので退けた．
- **Q2 送信の「流れていない」**：受け付けの空と，装置に設定を入れてよい空を分ける（§2.1 の 2・3）．後者の安全の条件は INFERRED で，実機で確かめる．UHD の `set_tx_rate` は DUC の input rate を書き換えるだけで，前の burst が出終わるのを待たない（VERIFIED，[multi_usrp_rfnoc.cpp:1999-2018](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L1999-L2018)，[duc_block_control.cpp:569-591](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/duc_block_control.cpp#L569-L591)）．
- **Q3**：時刻つきの更新だけのための能力と仕組みは消し，開始と送信に要る timing の能力は残す（§2.1 の 4）．
- **Q4 位相**：開始の前の設定は時刻なしの tune．UBX・OBX でも位相を保つことは約束しない（UBX は command time の有無で phase sync の設定を切り替える，VERIFIED，[db_ubx.cpp:949-958](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_ubx.cpp#L949-L958)）．owner の判断（2026-11-04）：sample の単位の位相・周波数の制御は信号処理で行う．repeat の周期のロック（sample の単位）と LO の位相の coherence は別の保証で，波形の差し替えだけなら retune しないので前者は保たれる．
- **Q5 Easy API の channel ごとの値**：logical channel の番号を key とする dict，`sample_rate` だけ stream の値（§2.4）．owner に見える形なので §6 の O3 で確かめる．
- **Q6 admission と「流れている」の順**：Kernel の順を変えない．順は，run の状態・target → 宣言された class → 値の種類 → Provider の `coerce` → 提案の configuration → 登録された admission の検査と coercion の方針 → admitted configuration の更新と dispatch → Provider の booking（stream の状態，予約，projection）．profile の外の rate は `coerce` で，RF envelope の外の周波数は admission の検査で，許される値で流れている stream への change は Provider の booking で拒まれる．最後のものは Kernel の log では admitted で，Provider の event になる（KC-27：admitted configuration には値が残る）．

## 4. 実装の段階（縦に切る．Sol 10）

各段階は AGENTS.md §9 の risky の手順で，両方の Provider，VH の生成と oracle，parity，Python を一緒に変え，tree を緑に保つ．古い系列を skip して緑にしない．対応する mutation の family も同じ段階で直す．

1. **初期の channel の組の道**：`connect` の時に組を決める入口（O1 の答え，stream の名前を含む）と fixture，例（`python/examples/minimal.py` など），server の default profile の試験．
   **済**（2026-11-04）：`coordinator::connect(profile, initial, assembly, lease)`，server の `initial::expand`（配置にない channel・stream は Run の前に拒む），Python の `connect(rx=, tx=, radio=)` と `ezsdr.Stream`，既定はどの channel も stream に入らない（RM-30）．Sol のコードレビューで major 2 件（Vocabulary の持ち主を prefix で決めていた，Spec の prefix の検査が Vocabulary の id を prefix と読んでいた〈前からの不具合〉，SB-2 の文を直した），minor 3 件，すべて直した．gate は J33 が生き残り（RS-19 の floor を殺していた `v58_13` が初期の道に移った），実行中の有効化のテストを足して付け替え，PASS．記録 `reviews/47s1-*`．
1b. **channel の名前**：selector の `#` の名前（RM-28），server の名前の解決と名前の表の受け渡し，Easy API の `str` の channel，MockRadio と UHD Module の selector の読み取り．
   **済**（2026-11-04）：`#` を読むのは `split_entry` の一か所，名前の規則は `name_error` の一つ（stream の名前と共有，Rust と Python は同じ `names.json` で試す），profile 全体の一意は `catalogue::assemble`．数は ASCII の十進の正規の綴りだけで，ほかの綴りは名前．名前だけの stream は名前を持つ radio へ移る．`connected.channels` が名前の表．**名前は binding の一部**（Kernel は selector を開かない．子の Run は親の selector を名前まで同じに書く．RM-28，RS-25a，orchestrator の裁定）．既定の profile には名前を付けていない（owner への問い）．Sol の計画のレビューで major 4 件・minor 2 件，コードのレビューは PASS．記録 `reviews/47s1b-*`．
2. **channel の組を Run の間固定**：両 Provider，VH の生成（`crates/ezsdr-radio/tests/generator/mod.rs:16-78`），parity（`crates/ezsdr-acceptance/tests/parity.rs`），Python，spec 07・09・16・18．move，from none，`Release` の holder を消す．
3. **受信の新しい設定と開始**：「流れていない」と staging，開始の snapshot．古い受信の仕組み（時刻つきの release，受け渡し，restore）をこの段階で消す．受信の rate の変更は，実機の確認まで拒む（受け入れの条件）．
4. **送信の空の判断，設定を伴う burst，clock の規則**：送信の rate の変更は，実機の確認まで拒む．**未決（再レビュー 5）**：設定を伴う burst の booking で新しい clock C を登録し，装置が設定を拒んで C が origin で終わる（RM-25）と，送信の stream に流れている SampleClock がなくなり，直した値の次の burst が Kernel の admission で拒まれる（`admission.rs:150-162`，`state.rs:336-339`）．Kernel の admission を変えずに，再試行が Provider の booking に届く道を，この段階の設計（brief とテスト計画）で決める．「失敗 → 直す → 送り直す」の系列をテスト計画に必ず入れる．
   **済**（2026-11-07．記録 `reviews/47s4-*`，扱いは `47s4-dispositions.md`）：送信の rate は拒んだままなので，値を伴う burst も新しい clock を登録しない（orchestrator R1．送信の clock は stream ごとに一つ，`arm` で登録．上の未決はこの段階では起きず，**送信の rate の変更を受け付ける段階で設計する**）．(a) B4（48）：送信の計画は一区間，cut は origin だけ．(b) 送信の `frequency_hz`・`gain_db`・`antenna` は `cold`．値は受付の時に stream が空いている時だけ（共有の `ezsdr_radio::staging::Busy`：予約の終わり，repeat は Stop まで，Stop の後は高々 in-flight window 一つ．引き渡しの進みでは決めない）．burst の予約が snapshot（値，generation，ActionId）を取る．UHD は uhd-tx の自分の turn で，開いた device burst がなく，自分の最後に閉じた device burst の終わりの後，recovery の外で，rate なしで時刻なしに設定し，その後 late policy で決め直す（実機 T5 の根拠）．装置の値は record の `configured`（generation）．設定の拒否はその burst を id つきで拒み，stream は止めない．MockRadio は最初の sample の時刻ちょうどから値を使う（B6）．`tx_start_lead_ns` は消した．Python の `repeat(…, frequency=, gain=, antenna=)`．Mock と Module が別の予約を受け付ける場合（#79）は `Busy` も分かれる：段階 5 で直す．
5. **重なりの拒否と repeat の差し替え**（両 Provider，parity，Python．`python/tests/test_easy_api.py:623-630` の「repeat を二度呼ぶ」試験も新しい規則に）．
   - **この段階の設計で問うこと**（owner 2026-11-06）：preemption が消えた後，送信の in-flight window は有限の burst に要るか，repeat の差し替えだけに限れるか．限れれば有限の burst は v3 のように頭から大きな send で渡し，開いた最初の send の最小（48 S3），preemption の前線，send の刻みとその試験を減らす．v3 は一回の長い burst（SOB，波形の残りを丸ごと send，0 サンプルの EOB）で送り，送信の async を読まなかったので，この種の装置の角を踏まなかった（`v3/cpp/uhd_usrp/multiusrp.cpp:591-647`）．
   - **決定（owner 2026-11-07）：有限の burst にも window を残す．** Stop で止まるまでの遅れの上限（RM-16，高々 `tx_in_flight_ns`）と，送り出しの時の late policy の判定し直しを保つ．window の仕組みは repeat の差し替えにも要る．消えるのは段階 5 が消す割り込みの前線だけ．退けた案：repeat の差し替えだけに限る（Stop の遅れが装置のバッファの深さ〈未測定〉まで延び RM-16 を改める），実機で測ってから決める．
   **済**（2026-11-07．記録 `reviews/47s5-*`，扱いは `47s5-dispositions.md`）：受付で決める予約の記録を一つ（`ezsdr_radio::staging::Bookings`，段階 4 の `Busy` を置き換え）にし，MockRadio と Module は呼ぶだけ．約束（RM-15）：受け付けた予約は，ホストの遅れか前の burst の configure の拒否（KL-38）がない限り，（late policy で動いた）開始で Module で鳴る．重なりは受付の順によらず拒む．装置の burst の区切り（頭が 32 サンプルを超える chain，前線 `start+5`／`end+1`／`end+2`），`J(e)` での閉じ（UHD の owner が streams の lock の下で），Stop の区間，渡した後の予約より前の予約の拒否（KL-37），置き場所の規則 `place`，差し替えの境目も記録が決める．preemption と `core.held` は消え，送り出しでの移動が他の予約に当たれば拒む．repeat の差し替え（RM-37，MR-42，UR-38）は `start_repeat` に印 `radio.repeat_operation = "switch"`（Python `Tx.switch`）．流れている stream への二つ目の repeat は拒む．#79 の違いはすべて parity の試験で両 Provider が同じ答え．
   - **実装中の owner の決定（2026-11-07）**：(i) 差し替えは別の verb を作らず印だけ（Kernel の compile 規則に metadata がないため）．(ii) 境目をもう渡した差し替えは拒まず late policy に従う（SendAsap は次の境目へ，`TIME_ERROR{send_asap}`）．(iii) 後の時刻の予約を先に受け付けて渡した後に来た，それより前の予約は受付で拒む（KL-37）．(iv) **時刻を指定しない burst** はその stream が取れる最初のサンプルから始め，`TIME_ERROR` を出さない（Kernel が `Dispatched.untimed` を Provider に渡す．時刻つきの SendAsap が Stop の区間だけに当たれば `TIME_ERROR{send_asap}` で動く）．原則（RM-15）：装置の都合で頼まれた開始に始められなければ late policy が決め，受け付けた別の予約とぶつかれば拒み，時刻なしは最初の空きに置く．
   - **この段階の後に**（owner 2026-11-07）：時刻と予約のモデルの棚卸し（各規則の約束・実機の証拠・なくすと壊れるもの・簡単な代案，利用者がほとんど使わない機能を削る候補は owner が一つずつ決める．controller が可変の予定を持ち確定したものだけを一方向の FIFO の executor に流す分け方を，速さではなく簡素化と責務の分離のために比べる）．
6. **使われなくなった共有の仕組みの整理**（`timed_instant`，`covering`，queue と release の能力，schema）．

**段階 2 以降の組み替え（orchestrator 2026-11-04．Sol が確かめて推した：`reviews/47s2-sol-restage.md`）**：段階 2 の試験計画で，組の変更をきっかけに使っていた残す道のテストが約 113 本，殺し役を失う mutation の行が 63 行あると分かった（`reviews/47s2-*`）．その多くは，段階 3・4 で消える仕組み（流れている stream や空の stream への `cold` の変更が開く from none，時刻つきの release，restore，instance の floor）を守っている．消える仕組みのためにテストを作り直すのは無駄なので，**消す段階を先に，足す段階を後に**並べ替える：
- **段階 2（新）：Run の中の radio の値の変更をいったんすべてやめる**．radio の key から update class を外す（RM-27 の class をすべて `—`）．Kernel の admission が Run の中の `SetParameter`／`UpdateParameter` と Spec の `schedule` の radio の更新を拒む（RS-52，SB-16）ので，Provider の中の変更の道（move，from none，release，時刻つきの release と受け渡し，restore，live `cold`，instance の floor，`hardware_timed` の queue）は届かなくなり，テストごと消す．受信は 2a，送信は 2b に分けてよい（どちらも tree は緑）．残すもの：初期の組と値（`connect`・Spec の `requires`），Stop と `start_rx`，burst，ちょうど続く burst，repeat，送信の recovery（F8），loss，Provider::stop，T3．§58 の 16（Run の中の envelope の外の値の拒否）は，この間は class のない key の拒否として試し，段階 3 で envelope の拒否に戻す．RS-19 の floor（J33）は Kernel の試験の Provider で origin が未来の clock を作って守る．
- **段階 3：受信の値を止めた stream に戻す**（`cold` の class を戻し，staging と `start_rx` の snapshot）．
- **段階 4：送信の値を空の stream に戻す**（設定を伴う burst，clock の規則，拒否の後の再試行）．
- 段階 5・6 は変えない．この間，Run の中で値を変える機能はなくなる（利用者はまだいない）．
- **Sol の確認で足したこと**：
  - 届かなくなるのは Provider の更新の入口だけ．`start_rx` の設定の実行，segment の refusal，Stop と cut，recovery，clock の操作は残る．
  - **消しても残す部品**：`RxChain` の segment の識別と寿命，cut と Stop，uncertain／lost，最初の sample の遷移，owner の設定の snapshot と refusal の halt（claim の返却，他の stream に波及しない），`Core::configure` と read-back の refusal（rate と antenna，rate の集合の比較），受信の開始の lead の規則（`timeline.rs` の cut・旧 rate の受信の呼び出し・余裕・lead・lattice）と開始の逃しの再試行，clock の登録と終わりの順，claim と read の排他，generation と report の対応，T3，F8，MockRadio の channel の値の履歴（遅れた sample は過去の値で計算する）．
  - **設定の完了の `ready` 時刻**は今は消える `cold` の道の中にある．実装は一時なくなってよいが，要件と試験の筋書きは段階 3 に残す（設定の時間が開始の lead を黙って食わないため）．
  - 2a（受信）と 2b（送信）は別々に緑にできる．方向に共通の仕組み（`book_cold`，from none，時刻つきの queue と floor，move の入れ物，profile の項）は，最後の呼び出し元が消える 2b で消す．
  - §58 の 16 は，この間は class のない key の拒否（拒まれ，log に残り，dispatch されない）として試す．envelope の検査は Run の始めの値で試し続け，Run の中の envelope の拒否は段階 3・4 で戻す（一時の穴として記録する）．
  - RS-19 の floor は Kernel の `SteppedProvider.with_arm_origin_after` と J32 がすでに守っている．J33 は J32 にまとめる．
  - H4 の制御の dispatch の代わりの測定は，時刻つきの gain の release を測っている．test の Vocabulary の key は正直な代わりにならないので，H4 のその観測は退け，送信の余裕と負荷の測定だけを残して範囲を書き直す．Run の中の rate の変更を前提にした測定の campaign も，この間は止めて段階 3・4 で設計し直す．
  - `KNOWN_LIMITATIONS.md` の該当の項（時刻つきの retune，組）を同じ変更で直す．
- **2a と 2b をまとめる（orchestrator 2026-11-04）**：受信の key だけ class を外すと，Kernel を通る試験が 271 本落ちた（`reviews/47s2-*`）．受信の machinery の多くは送信と共有で，2a で送信の key に付け替えた試験は 2b でまた消える．そこで **受信・送信の radio の key の class を一度に外し**，Run の中の値の変更の machinery を両方向まとめて消す．試験は，守る振る舞いが残るもの（Stop と `start_rx`，burst，続き，repeat，F8，loss，Provider::stop，T3，Run の始めの値と refusal）を初めの値の形に書き直し，Run の中の変更そのものを守るものは消す．送信の拒否の後の再試行（S10）は段階 4 で設計する．作業は (a) class を外して落ちる試験を一本ずつ裁く，(b) 届かなくなった本体とその中の試験を消す，(c) 行・spec・KNOWN_LIMITATIONS，の順．(a) の後に確かめ直しのレビュー，(c) の後に Sol のコードレビュー，gate は最後に一回．
  **済**（2026-11-05，2a と 2b をまとめて一度に）：radio の key はすべて class なし（RM-27），Python の setter と `.stream` は Run の中で `Error`，Provider は直に届いた class なしの更新を拒む（MR-18，UR-26）．Mock・UHD・共通の時間表から変更の道を消した（本体 約 2 700 行，テスト 約 320 本）．VH の生成器は Stop・StartRx・Fault・Loss の class だけ．`cold_change_refused` の fault は取り下げ（予定に書くと拒まれる）．消した試験の引用は `reviews/47s2-renames.txt`，gate は `--renames` を渡す．レビュー：Opus の確かめ直しで，(c1) が殺せないとして消した行のうち 4 行は生きた試験が殺し，15 行は守りが外れていた（試験を足して戻した．再開が connect の値を保つ試験も）．Sol のコードレビューで P2 1 件（loss か `Provider::stop` が送信の clock の origin より前で切ると clock が終わらない → 予約の時点で終わらせる道を戻した），P3 1 件（spec の古い記述）．記録 `reviews/47s2-*`．
  **段階 3・4 に残したこと**：`ready`（設定の完了の時刻）の筋書き（`fake.rs`@45a4294:2783），rx.rs の `start_rx` の拒否が `"update_parameter"` を名乗る誤り（`core.rs` の clock の end の失敗と RM-25 の book の誤りも同じ名前），§58 の 16 の Run の中の envelope の拒否，受信・送信の開始の lead の測定の群，`tx_start_lead_ns`（今は何も決めない），`CancelReason::ColdChange` の名前，Mock の `applied` の節（常に空），Vision §15・§27・§58 の 16 の文（段階 3・4 の後に直す），H4 の root tick の換算（P1148，実機の道具），UHD の owner の「同じ設定なら設定しない」比べ（`Owner::Nothing`・`configured` の印．今は届かないが段階 3 の値を変えた再開で要る），loss の後に控えた複数の fault の記録の順（Mock と照合の模型で違う．生成器の Loss は loss 以外の fault を一つまで）．

  **済**（2026-11-05．記録 `reviews/47s3-*`．gate は 1 回目に mutation の 3 行で落ち，直して PASS）：受信の `frequency_hz`・`gain_db`・`antenna` が `cold` に戻った．「流れている」は stream の line の最終の状態（`Line::streaming`，効く順に replay して `on`）で，到着順の旗ではない（`Stop` の後に `at` が先の `start_rx` があっても取り消さない）．staging は純粋な `ezsdr_radio::staging::Staging`（値の map だけ）で，両 Provider の呼び出しは配線だけ．`start_rx` は booking で snapshot を取る（MockRadio は `RxStream.snapshots`，UHD は `ColdConfig::of(snapshot, id)`）．設定の時間は開始の lead の中にあり，origin は動かない：設定が `min_timed_command_lead_ns` 未満しか残さなければ UHD の owner が `radio.START_LEAD_SHORT` を出す（`timeline::lead_left`），origin 以後に終われば今までの missed start．O2：`Submitted.refused`（その Action の id の `radio.COMMAND_REJECTED`），dispatch したのに Run が Running でなければ `ended`．Easy API は `rx_streams[k].start(at, frequency=, gain=, antenna=)` と setter（`Rejected` に Provider の event），rate と送信は `Error` のまま．§58 の 16 は受信の antenna の envelope の拒否と，流れている stream への許された値の Provider の拒否（action id で突き合わせ）．生成器に `Value` の class と `at` つきの `StartRx`，独立の oracle．試験計画と記録 [reviews/47s3-test-plan.md](reviews/47s3-test-plan.md)．**Sol のコードレビュー後の fix**（2026-11-05）：(1) 流れている stream への `start_rx` は両 Provider が拒む（`Staging::start`，kind `command`，その Action の id，line は変わらない．booking 済みでまだ始まらない start も「流れている」に数える）．これで booking 済みの start が早い origin に引き直されることはなく（Sol P2：stop → `start_rx(at=+1 s)` → `start_rx(at=+200 ms)` で uhd-rx の cursor が生き残った早い start を飛ばす），層 3 の差分は `at` つきの start を戻した（引き直しの再計算は段階 2 が消した仕組みなので足さない）．halt した stream も「流れている」ので，拒まれた start の後は `Stop` してから `start_rx`（RM-25，KL-35）．(2) Python は負の channel 番号を送る前に `Error`．(3) booking の失敗は dispatch された id を名乗る（`Submitted.refused` が拾う）．(5) 欠けた拒否の場面は fake が出す `LateStart` で SHORT→LATE を決定的に走らせ，最初の block の gap と lost を見る．(6) spec の誤りを直した．
  **残したこと**：受信の開始の lead の測定の群（compile だけの実機の道具）の復元は**名のある後続**（Sol P3．owner の bench の段階，作業順の ③ が，profile の `rx_start_lead_ns` を変える前に復元して走らせる．それまで実機は `START_LEAD_SHORT` を報告しうる．KL-35 に書いた），Vision §15・§27・§58 の 16 の文，送信（段階 4）．`Line` の replay の `halted` の枝は消した（halt した stream への `start_rx` は拒まれる）．`Line::halted` とその欄は今は試験だけが読む：段階 6（共有の仕組みの整理）で決める．

実機（報告 E4 §6 の 3 の 3 番目）は段階 3 と 4 の rate の受け入れの条件として，その段階の中か直後に置く（Sol 12）．確かめること：新しい rate，旧 burst の末尾，時刻，最初の sample，read-back，streamer を作り直さないこと．

**実機（2026-11-05，[bench-results](bench-results.md) §「2026-11-05」）**：段階 3 の道は二台（UBX，OBX）とも PASS（拒否と Action の id，予約した origin，read-back．再開の最初の約 13＋10 サンプルは装置が出す 0 と立ち上がり：FakeDevice と MockRadio で再現する）．**受信の rate の受け入れの条件は満たした**（開いたままの streamer で 1 → 2 → 10 → 1 MS/s，二台とも．受け付けるのは段階 4 以降の段階で決める）．送信の rate は #2 が PASS，#1 が INCONCLUSIVE で，まだ満たさない．§2.1 の 3 の送信の設定の安全の条件は二台で観測した：前の burst の ACK（毎回 e＋1 tick）の後の時刻なしの設定は，＋0〜＋20 ms のどれでも末尾を乱さず，e の 4.6 ms 前の設定はすぐ末尾を変えた（破壊的な対照）．受信の start lead は候補 3 ms でも遅れない．Module を通る送信の underflow（#78）が見つかり，owner は段階 4 の前に，この段階たちの理由がなくなった仕組みを棚卸しする設計メモ 48 と簡素化の段階を入れると決めた（handoff (7) ③′）．段階 6 の片付けはその簡素化に吸収する．

## 5. 消える範囲（Sol の洗い出し）

Sol の一覧（規則と名のあるテスト，本体の区間，mutation の行の family，ほかに直す試験）は [reviews/47-draft-sol.md](reviews/47-draft-sol.md) の「Removal inventory」にある．各段階の brief はそこから自分の範囲を引く．Sol の注意：広い ID の範囲や，名前に `cold`・`front`・`timed` を含むだけで消してはいけない（残る機能が使うものがある）．

## 6. owner に決めてもらうこと（一つずつ）

- **O1 channel の組をどこで決めるか**（Sol 11）：今の Session は，`connect` の後で `sdr.tx_channels[0].stream = 0` として送信の channel を stream に入れる（既定の layout では送信の channel はどの stream にもない，RM-30）．F4a (d) でこれができなくなるので，`connect` の時に決める入口が要る．
  **決定（owner 2026-11-04）：(a) と (b) の両方．** (a) `connect` に初期の設定を渡せるようにする（例 `ezsdr.connect(tx={0: [0]}, rx={0: [0, 1]})`）．Session の暗黙の Spec の初期の設定の要求になる（channel の組は intent なので Spec の側．BindingProfile には書かない）．周波数や rate の初期値も同じ道で渡せる．(b) 既定の layout で送信の channel 0 も stream 0 に入れる（RM-30．受信の channel 0 と対称）．確かめること：RM-31（duplex の board の受信の antenna）と既定の antenna が合うか．合わなければ既定の antenna も合わせる．退けた案：BindingProfile に書く（intent と binding の分け方に反する），(b) だけ（二つ以上の channel の Session が組めない）．
  **改め（owner 2026-11-04）：(b) を取り下げ，受信も送信も既定ではどの stream にも入れない．** 使う channel は `connect` で明示する（例 `ezsdr.connect(rx={0: [0]}, tx={0: [0]})`）．理由：(b) は spec 07 §6 の R2 が退けた案で，送信を指定していない UHD の Run でも `arm` で送信の streamer が開かれ既定の周波数と gain で設定される（頼んでいない送信機．LO の漏れなどの電波は INFERRED）．受信を送信に合わせて「頼んだ channel だけが stream に入る」一つの規則にする（owner の案）．RM-30 の既定と R2 の文を改める．確かめること：既定の profile が受信の port `rx0` を capture の sink に結んでいる（EA-9）ので，その channel が stream にない時に結び目が「何も流れない」になり `connect` が通るか．退けた案：初回の送信の前に Easy API が組に入れる（Run の途中の組の変更で F4a とぶつかる．streamer を開くのだけ遅らせる形は，F5 で消した「空の stream を開く」とほぼ同じ仕組みを呼び戻す），受信だけ既定のまま（規則が二つ）．
- **O2 Provider の拒否を Easy API でどう見せるか**（Sol 6）：Kernel の admission を通った後の Provider の拒否（流れている stream への change，重なる予約）を，例外にするか．
  **決定（owner 2026-11-04）：(a)．** server が submit した Action の id を持つ `radio.COMMAND_REJECTED`（RM-22 の `action`）を集めて submit の返事に入れ，Python はそれがあれば `Rejected` を投げる．根拠：装置の時間で進む Run では，Kernel は Action を渡した後 Provider が処理し終えるまで待つ（KC-21a，MA-14b）ので，submit が返る時には拒否の event が出ている（INFERRED）．確かめること：simulation の Run（MockRadio）でも submit が返る時に判断が済んでいるか．変えるもの：server の返事の frame と schema（EA-6，EA-11），Python の client．退けた案：(b) event と Manifest にだけ残す（流れている stream への設定が黙って成功したように見える），(c) 「流れている」の判断を Kernel の admission に移す（Kernel に radio の事情を持ち込む）．
- **O3 Easy API の値の渡し方**（Q5）：channel ごとの dict でよいか．
  **決定（owner 2026-11-04）：dict．** `frequency`・`gain`・`antenna` は logical channel の番号を key とする dict，`sample_rate` は stream の値．書かなかった channel は変えない．その stream にない番号は client が送る前に拒む．止めている stream には，今の channel ごとの属性（`rx_channels[n].frequency = …`）でも値を入れられる．退けた案：数値一つで全 channel（channel ごとに違う値を書けない），数値と dict の両方（形が二通りになり規則と試験が増える），並びの list（位置と番号を取り違えやすい）．

## 7. 見込み（INFERRED）

- 流れている間に時刻つきの値を装置に渡すことがなくなり，D9b-2・L1 の受け渡しの競合，#71・#72 の場合，一般の割り込みの境目の競合がなくなる．
- 新しく生まれる競合（Sol 4，5，9）は，§2.1 の staging の規則，送信の設定の適用の条件，差し替えの直列化で閉じる．どれも段階のテスト計画で系列として試す．

## 8. Sol のレビュー

2026-11-04，`openai/gpt-6.1-sol`，読むだけ，下書きに対して．12 件（DRAFT NEEDS CHANGES）．すべて取り入れた：差し替えの印（1），選んだ境目の記録（2），送信の clock の登録と origin（3），受信の「流れていない」と予約済みの開始（4），送信の受け付けの空と適用の空（5），Provider の拒否と Python（6，O2），(A) の部分的な失敗（7），区間での重なりの判断（8），差し替えの直列化（9），縦に切る段階（10），初期の組の入口と Vision の例（11，O1），rate の実機の確認を受け入れの条件に（12）．Q1〜Q6 の答えも採った（§3）．記録 [reviews/47-draft-sol.md](reviews/47-draft-sol.md)，ブリーフ [reviews/47-draft-sol-brief.md](reviews/47-draft-sol-brief.md)．

第 2 版の再レビュー（2026-11-04，同じ Sol）：前回の 12 件は 9 件が閉じ，3 件（6，7，11）が一部残った．新しい指摘 5 件（NOTE NEEDS CHANGES）．すべて取り入れた（5 は段階 4 の設計で決める未決として）．O1 の RM-31 との衝突はない（既定の受信の antenna は UBX・OBX・CBX とも RX2，`design/07-radio-model.md:299-301`）．O2 の前提は VERIFIED（装置の時間の Run：`pipeline.rs:1034-1038` の `wait_finished`，MockRadio の Session：`stepping.rs:159-181` の step）．段階 1 の範囲（規則，コード，テスト，例，テスト計画の系列 10 本）は記録の「段階1のscope」にある．記録 [reviews/47-sol2.md](reviews/47-sol2.md)，ブリーフ [reviews/47-sol2-brief.md](reviews/47-sol2-brief.md)．

## Changes

| 日付 | 変更 |
|---|---|
| 2026-11-04 | 下書き（orchestrator Opus，tree の外） |
| 2026-11-04 | 第 2 版：Sol の 12 件と Q1〜Q6 の答えを入れて tree に入れた |
| 2026-11-04 | 第 3 版：Sol の再レビューの 5 件（connect の初期の要求の契約と Kernel の Session の入口，組の map の意味，複合の呼び出しの止め方，読み返す値は admitted，送信の設定の拒否の後の再試行の道〈段階 4 で決める〉）を入れた．O2 の前提（submit が返る時に booking の判断が済んでいる）は，装置の時間の Run と MockRadio の Session の両方でコードで確かめられた（VERIFIED） |
| 2026-11-04 | 段階 1 を済とした（§4） |
| 2026-11-04 | 段階 1b を済とした（§4） |
| 2026-11-04 | 段階 2 以降の組み替えの案（消す段階を先に，§4） |
| 2026-11-04 | 段階 2 の 2a と 2b をまとめた（§4） |
| 2026-11-05 | 段階 2 を済とした（§4） |
| 2026-11-05 | 段階 3 を済とした（§4） |
| 2026-11-05 | 段階 3 の実装を記録した（§4） |
| 2026-11-07 | 段階 4 を済とした（§4） |
| 2026-11-07 | 段階 5 の問い（window）を owner が決めた（§4） |
| 2026-11-07 | 段階 5 済（§4）．実装中の owner の決定 4 件と，段階の後の棚卸しを記録 |
