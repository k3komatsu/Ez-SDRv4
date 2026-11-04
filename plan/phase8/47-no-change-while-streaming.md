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
   - **repeat の差し替え**（Q1）：`TxBurst { repeat: true }` に Vocabulary の metadata の印 `radio.repeat_operation = "switch"` を付けたもの（RM-13 の「metadata は空」を「空か，この印だけ」に改める）．新しい Session の verb（例 `switch_repeat`）がこれに compile する．印のない repeat は通常の repeat（重なれば拒む）．
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

1. **初期の channel の組の道**：`connect` の時に組を決める入口（O1 の答え）と fixture，例（`python/examples/minimal.py` など），server の default profile の試験．
2. **channel の組を Run の間固定**：両 Provider，VH の生成（`crates/ezsdr-radio/tests/generator/mod.rs:16-78`），parity（`crates/ezsdr-acceptance/tests/parity.rs`），Python，spec 07・09・16・18．move，from none，`Release` の holder を消す．
3. **受信の新しい設定と開始**：「流れていない」と staging，開始の snapshot．古い受信の仕組み（時刻つきの release，受け渡し，restore）をこの段階で消す．受信の rate の変更は，実機の確認まで拒む（受け入れの条件）．
4. **送信の空の判断，設定を伴う burst，clock の規則**：送信の rate の変更は，実機の確認まで拒む．**未決（再レビュー 5）**：設定を伴う burst の booking で新しい clock C を登録し，装置が設定を拒んで C が origin で終わる（RM-25）と，送信の stream に流れている SampleClock がなくなり，直した値の次の burst が Kernel の admission で拒まれる（`admission.rs:150-162`，`state.rs:336-339`）．Kernel の admission を変えずに，再試行が Provider の booking に届く道を，この段階の設計（brief とテスト計画）で決める．「失敗 → 直す → 送り直す」の系列をテスト計画に必ず入れる．
5. **重なりの拒否と repeat の差し替え**（両 Provider，parity，Python．`python/tests/test_easy_api.py:623-630` の「repeat を二度呼ぶ」試験も新しい規則に）．
6. **使われなくなった共有の仕組みの整理**（`timed_instant`，`covering`，queue と release の能力，schema）．

実機（報告 E4 §6 の 3 の 3 番目）は段階 3 と 4 の rate の受け入れの条件として，その段階の中か直後に置く（Sol 12）．確かめること：新しい rate，旧 burst の末尾，時刻，最初の sample，read-back，streamer を作り直さないこと．

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
