# Phase 8 — Mock ↔ X310 parity：概要と計画

| 項目 | 内容 |
|---|---|
| 状態 | **Gate P 受理（owner，2026-10-17「受理します．進めてください」）．** 判断 P1〜P6 と設計 33（第4版）の D1〜D16 と用語は §8．次は段階 1a（§5）．実機は owner が「揃った」と言うまで動かさない（handoff §2） |
| Phase | Vision §67 の Phase 8「Mock → X310 parity test」．前：Phase 7（Gate X 2026-10-01）．次：8b（複数台），8c（RFNoC） |
| 範囲 | §59 の parity test を作って実機で回す．実機の envelope を測り，**Mock の profile が測定より緩ければ失敗する**テストにする．profile の値は測定から決め直す．加えて，Mock が実機より緩いと分かっている箇所（#62，#51，#59，D2）と，Phase 7 から持ち越した測定（H1〜H5，#56，B8 の残り）を片づける |
| 範囲外 | §3．一言でいうと，複数台（8b），RFNoC（8c），spec 22 の手順3・5 |
| 言語 | 日本語（handoff と同じ）．`§N` と規則 ID はそのまま |

## 1. 何を示すか，何がもうあるか

Vision §59：「Mock 用に書いた実験は BindingProfile を変えるだけで USRP で動く」．Phase 7 はこれを X300＋OBX で示した（bench B0–B8）．Phase 8 は残りの半分を示す：

> parity test は実機の envelope（timed command の最小 lead，coercion の grid，start lead と再開の準備，restart gap，持続できる throughput）を測り，**Mock の profile が測定より緩ければ失敗する**．

Phase 7 の bench（[bench-results.md](../phase7/bench-results.md) Session 2 part 2）で，多くの値はすでに1回測っている．それを今の profile と並べると，**Mock の方が緩い値がもう見つかっている**：

| 値 | Mock `x310-like` | UHD `x310-obx` | Phase 7 の実測（X300＋OBX，1台） | Mock の方が緩いか |
|---|---|---|---|---|
| timed command の宣言 lead | 2 ms | 5 ms（device 2 ＋ 配送 3） | device は 0.5 ms 以上で 0/8 late，0.3 ms で 1/8（`hw_b8_raw_device_lead`） | **緩い**：lead 2〜5 ms の Spec が Mock で通り UHD で拒否される |
| timed command の queue | 16 | 16（Module が保持する数） | device は timed OBX tune を **7個**受けて8個目の `apply` が詰まる（`hw_b8_raw_queue_depth`） | **緩い可能性**：release window（3 ms）内に8個以上で uhd-control が詰まる（INFERRED，未測定） |
| 走っている burst の preemption | `now + 2 ms` | in-flight window（10 ms）の後 | 提出から約 10.4〜11.3 ms 要る（`hw_b8_preemption`） | **緩い** |
| timed update の順番どおりの release（#62） | 遅れない | queue の順で遅れうる（UR-24） | 未測定 | **緩い**（仕様上） |
| Stop で捨てる held burst の報告（#51） | 黙る | `COMMAND_REJECTED` | — | 報告が違う（D2 と同じ機構） |
| start lead | 50 ms | 50 ms | 受信は lead 1 ms でも要求した tick に始まる（`hw_b8_raw_restart_lead`）．送信は未測定 | 緩くない（Mock の方が厳しい） |
| overflow の restart gap | 50 ms | 50 ms | 単独では未測定（B5 は stall 込みで 456 ms） | 未確定 |
| 受信の throughput | 1e9 B/s | 1e9 B/s | 1ch 200 Msps を 3 s，取りこぼしなし（disk に書かない場合，`hw_perf_rx_rates`） | 1ch は緩くない．2ch と送信と disk 書き込みは未測定 |
| channel 数 | 0…2 | 0…1（OBX 1枚） | — | **OBX の実験から見ると緩い**（Mock は2ch を通す） |
| 周波数範囲 | 10 MHz〜6 GHz | 10 MHz〜8.4 GHz | UHD の source（VERIFIED） | 緩くない（Mock の方が狭い） |
| repeat の制約 | 256 Mi・偶数長 | 4 Gi・任意長 | — | 緩くない |

ここから分かること：
- parity の失敗は「測れば出るかもしれない」ではなく，**今すでに4件ある**．Phase 8 の仕事の半分は，その直しである．
- Phase 7 の値は1台・1回・負荷なしの測定である．profile の根拠にするには，2台・負荷あり・回数つきで測り直す．
- X310 の値は MockRadio（`crates/ezsdr-mock-radio/src/profile.rs`）と UHD Module（`crates/ezsdr-radio-uhd/src/profile.rs`）に**別々に書いてある**．両者の差は「UBX と OBX の違い」と「Module の配送 allowance」と「書き写しのずれ」が混ざっていて，コードからは区別できない（文書監査 D3）．

## 2. 機材と，それで言えること

owner が用意できるもの（2026-10-16）：**USRP X300 × 2，UBX × 1，OBX × 1**．UBX は壊れているかもしれない．同日の追記：**LFTX・LFRX・BasicTX・BasicRX・CBX も研究室にある**．どれも2枚ずつ用意できるが，他の実験で使っているので持ってくるのに手間がかかる．bench は board の付け替えをまとめて，借りる回数と枚数を最小にする（同じ種類の2枚は，要ると分かったときだけ）．

| 構成 | 言えること | 言えないこと |
|---|---|---|
| X300 #1 ＋ OBX（Phase 7 と同じ） | OBX の全値．motherboard と host の値（lead，restart，throughput 1ch） | 2ch の throughput |
| X300 #2 ＋ OBX（付け替え） | 個体差．profile は2台の悪い方に合わせられる | — |
| X300 ＋ UBX（動けば） | UBX の値（tune の queue 数，retune の整定など，board で変わるもの） | — |
| UBX が壊れていたら | UBX の board 依存の値は INFERRED のまま残る | `x310-ubx` を「測った profile」とは言えない |

- **最初の実機作業で UBX が動くかを確かめる**（§6 の BP0）．結果で §4 P1 の中身は変わらないが，`x310-ubx` を測るかどうかと，8b の準備が変わる．
- **8b への影響**：8b は2台の X300 にそれぞれ受信できる board が要る．UBX が壊れていると board が1枚しかなく，8b は始められない（UBX の修理か，board をもう1枚用意する）．早めに分かる方がよい．
- 2台目の X300 は FPGA image の版がまだ分からない．UHD 4.10 の image でなければ，書き換えるかどうかを owner に聞いてから決める．
- **X300 の背面には SFP+ が2つある**（owner の確認，2026-10-16）．UHD 4.10 の FPGA image は HG（既定：port 0 が 1 GbE，port 1 が 10 GbE）と XG（両方 10 GbE）で，2本目の 10 GbE は XG image と `second_addr` で使う（`host/docs/devices/usrp_x3x0.dox`，v4.10.0.0，VERIFIED）．Phase 7 の X300 #1 は HG image なので，今は 10 GbE が1本．今の profile の `rx_bytes_per_s`・`tx_bytes_per_s`（1e9 B/s）は 10 GbE 1本の値．
- そのため，**link の構成（image の種類，10 GbE の本数）は motherboard の description と selector と測定記録に入れる**（P1 (c) の spec で，共有資源の上限〈Sol の指摘7〉と一緒に決める．どの channel のデータがどの link を通るかは未確認）．2台 × 2本なら usrp-lnx03 の X710 の4ポートをすべて使う．

## 3. 範囲

### 範囲内

1. **X310 の description を1か所にまとめ，channel と stream を分ける**（P1 (c)）．MockRadio と UHD Module は同じ名前の profile を同じ値で使う（文書監査 D3 はこの構造で片づく）．混在 board（UBX＋LFTX＋BasicRX など）と，設定の違う channel を1本の aligned stream で受けることを表せるようにする．独立した amendment spec にする（設計：[33-channels-and-streams.md](33-channels-and-streams.md)）．
2. **parity test**（P2）：測定記録と，実機なしで毎回回る比較テスト．各値に「どちら向きが緩いか」を宣言する（文書監査 D10）．
3. **測定スイート**：`hardware.rs` の `hw_b8_raw_*` などを，記録を出す測定にまとめ直す．H1〜H5，#56，B8 の残り，送信側の start lead，overflow の restart gap 単独，送信と 2ch の throughput（測れる範囲），coercion の read-back．FakeDevice でリハーサルしてから実機で回す．
4. **Mock が緩い既知の箇所を直す**：#62（in-order release），#51 と D2（イベントの報告規則．先に設計メモ，AGENTS.md §6 の three strikes），#59（read-back 拒否の fault），preemption の bound，queue depth．
5. **方向別の start lead**（owner 決定 2026-10-08．未実装）：`radio.timing.start_lead_ns` を受信・送信の2つの値にする．
6. **計画32 の A2**（RM/MR/UR の 14 条件）と **B**（4件，実機が要る）のテスト．多くは 4 の変更で書き直すので，4 の後に書く．
7. **ホスト設定の script**：`ibfd_sefdm/set_addr.sh` をこのリポジトリへ移す（handoff §6.3）．
8. **測り直す持ち越し**：NB-5（TimingEnvelope が受信の `cold` lead を公開していない），UR-17 の受信要求長の上限（handoff §3）．
9. **測定から profile の値を決め直す**（P3）．Phase 7 の INFERRED の値（lead，allowance，windows）を測定値に置き換えるか，測れなかったと記録する．

### 範囲外

| 項目 | 理由 | 行き先 |
|---|---|---|
| 複数台，PPS での同時開始，v61_04 | Vision §67：8b は1台の測定の上に立つ | 8b |
| RFNoC Replay，DDC/DUC，device FFT | | 8c |
| spec 22 の手順3（spec 11 を 08 へ）・手順5（spec 10・14・16） | 文書の形式の変更で，parity と関係しない | Phase 8 の後（手順3の取りやめは owner に確認） |
| UR-32 の受信の per-call allocation の除去 | throughput の測定で律速と分かったときだけ直す（§59 が要るのは throughput の値で，この実装ではない） | 測定次第 |

## 4. 判断（Gate P で owner が決める．1件ずつ諮る）

| # | 問い | 推奨 | 退けた案（各1行） |
|---|---|---|---|
| P1 | Mock は何と比べるか，混在 board をどう表すか | **(c)（owner 2026-10-16）：description を `radio` 語彙に1か所だけ置き，MockRadio と UHD Module が同じ名前・同じ値で使う．device tree は channel の node と stream の node に分ける．** channel の node は front end 1つ（UBX の RX 側，BasicRX の frontend 0 など）で，周波数・利得・antenna を channel ごとに持ち，capability は board の種類の description から来る．stream の node は同じ rate の channel の集合で，SampleClock と aligned block は stream ごと（Vision §25 の CoherentGroup）．motherboard（`x3xx`：X300/X310 共通）の値は共有．profile 名は `x3xx` の1つ，slot と front end の構成は selector に書き，開くときに実物と照合する．board の種類は語彙の表に1行ずつ（ubx・obx・cbx・basic_rx/tx・lf_rx/tx）．`x310-like` と `x310-*` は改名する（破壊的変更は可，AGENTS.md §6）．Mock の lead などは UHD Module から見た値（配送 allowance 込み）．Kernel は1か所（設計 33 D2，P6）．詳細は amendment spec で決める（Sol の指摘の一覧：[reviews/p1-sol-review.md](reviews/p1-sol-review.md) の「設計メモで決めること」） | (a) channel ごとのキー：キーと Session の操作が channel 数だけ膨らむ．(b) 設定を共有するグループごとに node：BasicRX の A/B を表せず，設定の違う channel を1本の aligned stream にできない（Sol の P0）．構成ごとに profile 名（`x310-obx`，`x300-obx`…）：名前が掛け算で増える．1台を複数の Provider instance で分ける：所有・時刻・cleanup が分かれ，SB-23 と 8b に逆行する．slot ごとの node：BasicRX の中の独立設定を表せない |
| P2 | parity test の形 | **測定記録（JSON，`ezsdr-radio` crate の中）と，実機なしで `cargo test` が毎回回す比較テスト．** (1) 記録は bench 1回 × 1台 × 1構成に1つ．装置の serial，slot と front end の構成，mode，stream ごとの channel と rate，同時に動かした stream，UHD・FPGA image の版，ホスト設定，負荷の有無，commit と，値ごとに部品（motherboard か board）・回数・最悪値・測ったテスト名．測れなかった値は「未測定」と書く．(2) envelope の値ごとに「厳しい向き」を語彙で宣言する（文書監査 D10）．(3) 比較テストは，記録の構成（board，link の本数，rate，同時に動かした stream）をその値の**適用条件**として使い，同じ条件の profile の値が記録の最悪値より緩くないこと（rate の集合や antenna の集合など数値でない値は包含で比べる），envelope のどの値も記録に「測定」か「未測定」で現れること，向きの宣言がない値がないこと，を確かめる．(4) 数値で表せない振る舞いの違い（#62 の順番，#51 のイベント）はこのテストの対象外で，VH-8 の harness（Mock と UHD Module を同じ入力で比べる）と bench で見る．(5) FakeDevice のリハーサルも同じ形式の記録を出すが，比較テストは fake の記録を拒否する | 比較を実機テストの中だけで行う：実機がない日は回らず，profile を書き換えても測定とのずれに気づかない．記録を bench-results.md の文章だけにする：機械で比べられない．Mock が実行時に記録を読んで値を決める：最新の1回で profile が黙って変わり，丸めや余裕の判断（P3）とレビューを通らない |
| P3 | 測定から profile の値をどう決めるか | **値の種類ごとに読み方を決め，2台・負荷あり（H4）の最悪値を厳しい側へ丸めて profile にする．** (1) 読み方：「最小の lead」型（command lead，preemption，start lead）は N 回すべて成功した最小の段；「容量」型（queue depth，throughput）は全試行で持ちこたえた最大値；「時間」型（restart gap，cold change の途切れ，startup latency）は観測した最大値．(2) 回数の目安は段の確定に各 100 回，時間の値に各 20 回以上，throughput は 60 s 以上（具体の数は bench.md）．(3) 余裕は既定で足さない．足すときは値ごとに理由を記録に書く．(4) 今の値が測定よりずっと厳しいとき（start lead 50 ms に対し実測 1 ms）も測定値まで緩める：厳しすぎる Mock は実機で動く実験を拒否する．(5) 測れなかった値は据え置き，INFERRED のまま，記録に「未測定」．(6) Module 内部の定数（in-flight window，release window）は profile の値ではない．測定は参考にし，変えるなら Module の設計変更として理由とテストを付ける．Mock はそれらを UHD Module から見た振る舞いとして真似る（配送 allowance と同じ．P4 の (4)）．(6b) **測った境界**：記録の各値に「Provider 経由で測ったか，Device を直接叩いたか（raw）」を書き，比較テストと profile の値には Provider 経由の値だけを使う（raw は参考．例：Phase 7 の restart lead 1 ms は raw の値で，50 ms の理由は uhd-rx の loop だった〈`plan/phase7/bench-results.md:399-400`〉）．(7) 値はこのホスト（usrp-lnx03，§6.3 の設定）でのもので，ほかのホストでの保証はしないと記録する | 平均値や中央値：半分の回で実機が Mock より厳しくなる．一律の安全係数（×2 など）：根拠のない数が残り，実機で動く実験まで拒否する．今の値を据え置く：lead・preemption・queue の Mock が緩いままになる |
| P4 | #62・#59・#51 と，Mock が緩いほかの箇所をいつ直すか | **実機の前，段階2で．** (1) **#62**（Mock が device queue の順番どおりの release を模擬しない）：§59 の定義で parity の失敗なので直す．VH-8 harness が除外している `timed` の 25 系列を戻す．(2) **#51 と D2**（Stop で捨てる burst の報告，RX_OVERFLOW の `restart_gap_ns`・`lost`・件数が Mock と UHD で違う）：`mech:radio-rule-implemented-twice` は 9 件で three strikes を超えているので，直す前に設計メモを書く．問いは「イベントの報告規則も `ezsdr_radio::timeline` のように語彙で1か所に決め，両 Module がそれを呼ぶか」．(3) **#59**（read-back 拒否の fault）：小さな追加なので入れる．bench では拒否が実際に何回起きたかも数える．(4) **preemption の bound**：Mock も UHD Module と同じ「in-flight window の後」で判定する．(5) A2 の 14 条件のテストは (1)〜(4) の後に書く（多くが (2) で変わる）．順番は P1 (c) の spec の後（stream ごとのイベントの帰属が (c) で決まるため） | 実機の後に直す：測定のたびに既知の不一致を差し引いて読むことになり，測定スイートのリハーサルも Mock との比較ができない．Phase 8 の外に出す：§59 の「Mock は実機より緩くない」が閉じない．#51 を設計メモなしで個別に直す：three strikes の規則（AGENTS.md §6）に反し，同じ機構の次の不一致を生む |
| P5 | 2台目の X300 を Phase 8 で使うか | **使う．2台を usrp-lnx03 の別々の 10 GbE port に同時につなぎ，装置は `EZSDR_UHD_ARGS` で切り替える．** (1) motherboard の値（lead，restart，throughput など）は両方で測り，profile は悪い方に合わせる．(2) motherboard の値はどの board でも測れる（P1）ので，2台目には OBX を移さず，別の board（UBX が動けば UBX，なければ Basic/LF）を挿す．board の付け替えと借り出しが減る．(3) 2台で motherboard の値が目立って違ったときだけ，同じ board を両方に挿して個体差と board の差を切り分ける．(4) 2台目の FPGA image が UHD 4.10 のものでなければ，書き換える前に owner に聞く（BP0）．(5) 2台目の状態（probe，reference の lock，10 GbE）がここで分かるので，8b の準備にもなる | 1台だけで測る：1個体の値が楽観的でも分からない．2台目は確認だけ：motherboard の値の個体差が profile に入らない．2台とも OBX で測る：OBX を毎回付け替えるか2枚目を借りることになり，owner の手間が増える |
| P6 | Kernel の変更（設計 33 D2） | **`KeyDecl` に番号の付く segment を1つ持たせ，`registry.key_decl` が完全一致の次にそれで引く．** Phase 8 で唯一の Kernel の変更．KeyDecl の参照はすべて `registry.key_decl` を通るので，SB-2 の5つの位置，SB-45/46，KC-24 の step 5・6・11 をまとめて覆う（Opus が確認）．GPIO（Vision §39）にも使える | Vocabulary が `c0`〜`cN` を列挙する（Kernel は不変，上限は凍結後も足して上げられるが，上限の決め打ちと千を超える key）．番号を値に入れる（SB-5 の scalar に反する） |

P1 が決まると P2〜P3 の細部（記録をどの profile 名で持つか）が決まるので，P1 から諮る．

## 5. 手順

AGENTS.md §9 の分担で進める（実装は DeepSeek，セルフレビュー → Opus のレビュー → Sol の独立レビュー → 修正 → gate）．各段階で tree は緑のまま．

| 段階 | 中身 | 実機 |
|---|---|---|
| 0 | この計画，P1 (c) の amendment spec，Opus と Sol の独立レビュー，**Gate P**（P1〜P5） | なし |
| 1a | P1 の前半：description を1か所へ，`x310-like` の改名（既定 profile・fixture・Python・Vision を同時に），方向別の start lead，rate の格子の修正と CBX の `random`（値は段階4まで変えない，の例外：誤りと分かっている値），tree は 1b を妨げない形にする（設計 33 §5） | なし |
| 1b | P1 の後半：channel の node と stream の node，channel ごとの設定，複数 stream，混在 board（Spec の matching，Session の宛先，共有資源の上限を含む） | なし |
| 2 | #51 と D2 の設計メモ → 実装．#62，#59，preemption の bound．A2 のテスト | なし |
| 3 | P2：測定記録の形，比較テスト，測定スイート（H1〜H5，#56 を含む），FakeDevice でのリハーサル，ホスト設定の script | なし |
| B | **bench**（§6）．owner の準備が整ってから | あり |
| 4 | P3：測定から profile の値を決め直す．保守表の3・4（H1・H5 の後に決める），計画32 の B | なし |
| X | 出口の確認，**Gate X** | — |

段階1〜3 は実機なしで終わる．bench の時間には測定スイートを流すだけにする（handoff §2）．

## 6. bench（段階 B）

手順の本体は段階3で `plan/phase8/bench.md` に書く．RF の安全は Phase 7 の [bench.md](../phase7/bench.md) と同じ（OBX の TX/RX → 30 dB 以上の減衰器 → RX2，利得 0 dB，振幅 ≤ 0.5）．

| 手順 | 中身 |
|---|---|
| BP0 | **機材が揃いしだい（software は要らない）**．2台の X300 と2枚の board を `uhd_usrp_probe` で確かめる．**UBX が動くか**（probe に出るか，短い loopback で音が聞こえるか）．2台の FPGA image の版と種類（HG か XG か） |
| BP1 | ホスト設定（段階3の script，sudo は owner）．UHD のログで `Maximum frame size` を確かめる |
| BP5 | **BP2 の前に**，Phase 7 の B3〜B7 を新しい profile 名と書き直した Module で回し直し，まず動くことを確かめる（§59 の「profile を変えるだけ」の確認） |
| BP2 | 測定スイートを X300 #1 ＋ OBX で．H4 の負荷あり・なし |
| BP3 | X300 #2 に OBX 以外の board（UBX が動けば UBX，なければ Basic/LF）を挿し，motherboard の値を測る（P5）．2台の値が目立って違うときだけ同じ board で比べる |
| BP4 | UBX が動けば，X300 ＋ UBX で board 依存の値 |
| BP4b | 混在構成（例：slot A に UBX，slot B に LFTX＋BasicRX）で，channel ごとの設定・rate の違う複数 stream・設定の違う channel の aligned stream を確かめ，同時に動かしたときの lead・restart・throughput を測る．単独の測定を混在の同時運転の証明にしない（Sol の指摘9） |
| BP4c | XG image と 10 GbE 2本で，throughput（設計 33 D9 の「合計 ≤ 1本」が実機より緩くないこと，2本で何が出るか），再 open の後，受信と送信を同時に動かしたときの lead・restart を測る．image の書き換えは実施前に owner に確認する |

実機で失敗したらその場でコードを直さない．記録 → FakeDevice で再現するテスト → 修正，の順（handoff §6.3）．

## 7. 出口条件

1. 比較テスト（実機なし）が通る：どの profile の値も，その測定記録より緩くない．
2. 測定記録が揃っている：X300 2台（P5 の board の組み合わせ），負荷あり，各値の回数，UHD の版，FPGA image，ホスト設定，link の本数，構成，commit つき．測れなかった値は「未測定」と書いてある．未測定の構成を「実測で parity を確かめた」とは呼ばない．
3. X310 の値が MockRadio と UHD Module の片方だけに書かれていない（両者が同じ description を読む）．
4. #51，#59，#62 が閉じ，D2 が片づき，A2 の 14 条件にテストがある．H1〜H5 と #56 の結果が記録され，保守表の3・4 が決まっている．
5. BP5 の回し直しが通り，BP4b（混在構成）と BP4c（XG）が記録されている．
6. いつもの gate：`cargo test --workspace`，clippy（host と `--features uhd`），Python（3.14 と 3.9），`remote_mutate.py all`，`rules.py`．

## 8. 判断ログ

| 判断 | Gate | 結論 | 記録 |
|---|---|---|---|
| P1：Mock の比較相手 | P | **推奨（改訂版）で受理** | owner 2026-10-16「P1はそれでOK」．最初の案（構成ごとの profile 名）に owner が「x310-obx と x300-obx のように選択肢が爆発しないか」「2枚挿しはどうなるか」と問い，motherboard `x3xx` ＋ board の種類の2部品・枚数は selector・混在は拒否，に改めた．**同日，混在の拒否を取り下げて再検討中**：owner「片方に UBX，もう片方に LFTX/LFRX，さらに UBX＋LFTX＋BasicRX もよくやる」．今の `radio` 語彙（RM-4）は方向ごとに周波数・利得・antenna を1つしか持てず，混在をそもそも表せない（v3 は channel のグループごとに設定できた：`v3/docs/v3.md` の `tx-channels` / `for-channels`） |
| P1 の再検討：混在 board の表し方 | P | **(c) で受理** | orchestrator の案 (b)「設定を共有するグループごとに device node」を，owner の依頼で Sol 6.1 が独立レビュー（[reviews/p1-sol-review.md](reviews/p1-sol-review.md)，依頼文 [reviews/p1-brief.md](reviews/p1-brief.md)）．結論は「変更つきで採る」で，P0 が2件：BasicRX の A/B 独立受信は (b) の「グループ内で antenna 共通」と両立しない；設定の違う channel は (b) では別 stream になり，1本の aligned stream（Vision §25 の CoherentGroup）にできない．orchestrator が RM-5，SB-3，matching，RM-9，Vision §25 の引用を照合し，一致を確かめた．分かれ目は「設定の違う channel を1本の aligned stream で受けたいか」．owner 2026-10-16「あります．(c)で進めてください」：**channel の node（channel ごとの周波数・利得・antenna と front end の capability）と stream の node（同じ rate の channel の集合，SampleClock は stream ごと）を分ける**．独立した amendment spec にし，実装は description の共通化と，混在・複数 stream 対応の2段階に分ける |
| P2：parity test の形 | P | **推奨で受理** | owner 2026-10-16「P2はそれでOK」．記録の単位は (c) に合わせて「1台 × 1構成」（slot と front end の構成，mode，stream ごとの channel と rate，同時に動かした stream を記録） |
| P3：測定から profile の値を決める規則 | P | **推奨で受理** | owner 2026-10-16「P3はそれでOK」 |
| P4：#62・#59・#51 と Mock が緩いほかの箇所 | P | **推奨で受理** | owner 2026-10-16「P4はそれでOK」．段階2で，P1 (c) の spec の後．#51 と D2 は設計メモが先 |
| P5：2台目の X300 | P | **推奨で受理** | owner 2026-10-16「これでOK」 |
| 10 GbE 2本（XG image） | P | **扱う** | owner 2026-10-16「XGを使うこともあります」．link の構成（HG/XG，`second_addr`，本数）を motherboard の description・selector・測定記録に入れ，2本の構成も profile と bench で扱う．image の書き換えは実施前に owner に確認する |
| USRP 全機種の制約調査 | P | **推奨で受理**（owner 2026-10-16「これですすめてください」） | owner の依頼で sonnet-researcher が2本：X300 の詳細（[research/x300-uhd-facts.md](research/x300-uhd-facts.md)）と，USRP2・N2x0・X3x0・X410・X420・X440 に対する v4 の制約（[research/usrp-family-limits.md](research/usrp-family-limits.md)）．orchestrator の照合：DDC の decimation の集合が `{1} ∪ {2^h·c}` で作られること（`host/lib/rfnoc/ddc_block_control.cpp`，v4.10.0.0）を確かめた．値（halfband 3，CIC 255）は FPGA から読むので bench で確かめる．UR-3 の「mcr < 2^28」は spec 18 の `design/18-uhd-radio.md:106` にある（この行の初版は「ない」と誤っていた，Opus の P2-10 で訂正）．X4xx の master clock はこれを超えるので，X4xx を入れる前に round trip の議論を見直す．(c) の spec で今決めることとして7件を推す：master clock と FPGA image を description の入力にし X3xx の rate の格子を直す（今の格子は 200e6/N の N=257〜511 の奇数を通すが DDC は作れない：Mock が緩い），update class を device ごとに（X410・X440 は timed tune できない），node の id に motherboard を含める（8b），共有資源と link ごとの上限，RM-19 の RF envelope を channel ごとに，channel 間の拘束（UBX の TX/RX port，TwinRX の LO と antenna），1台に複数の root（X420・X440）．推奨：**形はこれらを表せるように決め，値を入れるのは手元で測れる X3xx の board（UBX，OBX，CBX，Basic/LF）だけ**．ほかの機種と board は表に行を足す形で後から |
| 複数 timekeeper（X420・X440）の表し方 | P | **推奨で受理**（owner 2026-10-16「その形で進めてください．計画ができたらSolにレビューしてもらってください」） | owner 2026-10-16「X420 とかの TimeKeeper を複数持てるやつは MockRadio × 2 とかで表現するのはだめなの？」．orchestrator の推奨：**timekeeper 1つを device node 1つとし，1つの Provider instance が複数の device を持てる形（SB-23，8b の複数台と同じ形）にする**．X420 は「device 2台の instance」，2台の X300 を 8b で束ねるのと同じ扱いになり，「1台に複数の root」という別の概念は要らない．別々の Provider instance（binding を2つ）にはしない：1台の装置を UHD で2回開くことになり，instance をまたぐ coherence は Kernel が推論しない（SB-35）ので，daughterboard をまたぐ aligned stream が作れない．stream は同じ instance の device をまたいでよい（Provider が alignment を宣言するとき）．root を共有するかは 8b の設計で決める（2つの timekeeper は同じ PPS で合わせて lockstep，UHD の X4DOX）．X440 の dual-rate（daughterboard ごとに別の master clock）が1つの root に載るかは推測（rate の比が有理数なら SampleClock の比で表せる），X440 を使うときに確かめる |
| 計画と設計 33 の独立レビュー | P | **全13件を採った．owner の判断待ち3件** | owner の依頼で Sol 6.1 がレビュー（[reviews/plan-sol-review.md](reviews/plan-sol-review.md)，依頼文 [reviews/plan-brief.md](reviews/plan-brief.md)）．結論「変更つきで進めてよい」，P0 1件（10 GbE 2本の上限が link の偏りで実機より緩い），P1 12件．orchestrator が引用（SB-4，RS-52，timeline，UHD の link の選び方）を照合し全件を採った：扱いは設計 33 §8．計画側は P2 の比較に適用条件，BP3 を P5 に合わせ，BP4c（XG・再 open・送受同時）を追加，出口条件2を直した．owner の判断待ち：設計 33 の D1（(c) の文面との違い），D9（XG の上限は 8c まで1本分），D13（CBX の `random` を範囲に戻す） |
| 設計 33 D1：設定の置き場 | P | **推奨で受理** | owner 2026-10-16「D1はこれでOK」．同時に「aligned と coherent を分けた」に対し「追加で shared_lo が必要．X420 などは LO 共有があり，位相が揃うより強い」：設計 33 D13 を「位相の関係」に広げ，aligned・coherent（timed）・shared LO の3段にした（Vision §25 の `basis` の `lo_sharing`）．Phase 8 の board は LO を共有しないので形だけ決める |
| 設計 33 D9：10 GbE 2本の上限 | P | **推奨で受理** | owner 2026-10-16「D9はそれでOK」．Phase 8 は方向ごとの合計を link 1本分以下，link ごとの指定と判定は 8c（UHD の RFNoC の C++ API）．BP4c で2本の実測を記録する |
| 設計 33 D13：位相の関係 | P | **推奨で受理** | owner 2026-10-16「D13はそれでOK．Gate Pに進んでください」．`radio` 語彙に phase `random` と channel ごとの `lo_group` を足し，stream の位相の関係を aligned・coherent・shared LO の3段で channel から導く．CBX は `random`．範囲外の表から phase `random` を外した |
| 設計 33 第2版の再レビューと独立レビュー | P | **採否を決め，第3版にした** | Sol 6.1 の再レビュー（[reviews/rereview-sol.md](reviews/rereview-sol.md)，依頼文 [reviews/rereview-brief.md](reviews/rereview-brief.md)：条件つきで Gate P へ，P1 4件．1回目の起動は4分半 0 byte で止まり，起動し直した）と Opus の独立レビュー（[reviews/rereview-opus.md](reviews/rereview-opus.md)：条件つきで Gate P へ，P0 1件・P1 11件・P2 12件）．どちらも受理済みの判断を覆さない．採否は設計 33 §9（P1-5 は D16，P1-10 は P6 として owner に諮る．P2-1 は記録）．計画側：P1 の「Kernel は変えない見込み」を訂正，P3 に測った境界，1a の行，範囲に NB-5 と UR-17，BP0 を早く・BP5 を BP2 の前に，出口条件に BP4b・BP4c，UR-3 の記録の誤りを訂正 |
| P6：Kernel の変更（設計 33 D2） | P | **推奨で受理** | owner 2026-10-17「P6はそれでOK」 |
| 設計 33 D16：antenna を `cold` に | P | **推奨で受理** | owner 2026-10-17「推奨でOKです」 |
| 用語：root は instance 全体（設計 33 §3） | P | **推奨で受理** | owner 2026-10-17「その理解でOKです．これらの追加の判断についてもSolにレビューしてもらってください」．`radio.device` は instance 全体を表し，timekeeper は `t<i>`（`radio.timekeeper`）．timekeeper を2つ持つ instance の1つだけを Spec が binds することはできない |
| 第3版と追加の判断の Sol レビュー | P | **全件を採り，設計 33 を第4版にした** | owner の依頼で Sol 6.1 がレビュー（[reviews/v3-sol.md](reviews/v3-sol.md)，依頼文 [reviews/v3-brief.md](reviews/v3-brief.md)）：条件つきで Gate P へ，P0 なし，P6・D16・用語はすべて賛成．P1 4件（halt と空の stream の復帰，antenna の read-back，開くときの利得の照合，流れていない DDC/DUC への command の条件）と P2 3件を採った（設計 33 §10）．D16 の理由（antenna は timed にできない）は原典と違ったので訂正し，`cold` は維持 |
| **Gate P** | P | **受理** | owner 2026-10-17「受理します．進めてください」．この計画（P1〜P6，§5 の段階，§6 の bench，§7 の出口条件）と設計 33 第4版（D1〜D16，§3 の用語）．レビュー：Sol 6.1 が3回（第1版，第2版，第3版），Opus が1回（第2版），指摘はすべて扱いを記録（設計 33 §8〜§10） |
| 段階1a の実装 | 1a | **完了**（2026-10-17） | DeepSeek が実装（セルフレビュー3回），Opus のレビュー（[reviews/1a-opus.md](reviews/1a-opus.md)：major 3・minor 3）と Sol 6.1 の独立レビュー（[reviews/1a-sol.md](reviews/1a-sol.md)：major 4・minor 4．3件は Opus と同じ）を全部採った（[reviews/1a-fix-brief.md](reviews/1a-fix-brief.md)）．主な修正：CBX の位相は timed retune でも保たれていた（実機より緩い）→ retune のたびに引き直す；layout の slot と UHD の channel が食い違えた → 1a は既定の順の prefix だけ；`rx: []` でも受信 1 channel が既定で開いた；CBX が `coherent` を名乗っていた；mutation R11 の条件が逆．Sol の再レビュー（[reviews/1a-sol-rereview.md](reviews/1a-sol-rereview.md)）は全件修正済み・新規なし．`rules.py` の改名は `--renames`（3件）で検査．Vision §8 の例の `instances: 2` は 1b-5 で直す |
| 段階1b-1 の実装 | 1b-1 | **完了**（2026-10-17） | DeepSeek が実装（セルフレビュー3回），Opus のレビュー（[reviews/1b1-opus.md](reviews/1b1-opus.md)：minor 4）と Sol 6.1 の独立レビュー（[reviews/1b1-sol.md](reviews/1b1-sol.md)：major 1・minor 2．major は Opus の O2 と同じ）を全部採った（[reviews/1b1-fix-brief.md](reviews/1b1-fix-brief.md)）．主な修正：Vocabulary の prefix の外の宣言を登録で拒否する（MA-34．残すと他の Vocabulary の正当な宣言を重なりとして誤って拒否した）；重なりの定義の文を実装に合わせた；固定 segment の負例．設計 33 D2 の「完全一致の次に番号付きで引く」は，登録が重なりを拒否するので当たる宣言は高々1つで順序が観測できず，`key_decl` は1回の探索にした．Sol の再レビュー（[reviews/1b1-sol-rereview.md](reviews/1b1-sol-rereview.md)）の minor 2件（綴りの文，拒否の後の同じ registry での登録）は orchestrator が直した |
| 段階1b-2a の実装 | 1b-2a | **完了**（2026-10-17） | 1b-2 をレビューの量のために 1b-2a と 1b-2b に分けた（[設計 33](33-channels-and-streams.md) §5）．DeepSeek が実装（セルフレビュー3回），Opus のレビュー（[reviews/1b2a-opus.md](reviews/1b2a-opus.md)：major 1・minor 2）と Sol 6.1 の独立レビュー（[reviews/1b2a-sol.md](reviews/1b2a-sol.md)：major 1・minor 2）を全部採った（[reviews/1b2a-fix-brief.md](reviews/1b2a-fix-brief.md)）．主な修正：全二重と throughput を `coerce` の中で既定値に重ねて判定していたので，Session の1 key の更新がすり抜けた → 設定全体を判定する公開の `Description::violations` を1つ置き，Provider は更新の時点の設定をそれで判定し直す（MR-18 の役目．配線は 1b-3・1b-4）；throughput のテストが受信の stream 0 だけだった．Sol の再レビュー（[reviews/1b2a-sol-rereview.md](reviews/1b2a-sol-rereview.md)）の minor 3件は orchestrator が直した．Basic/LF の周波数の上端と位相は INFERRED（spec 07 §8，bench で確かめる） |
| 段階1b-2b の実装 | 1b-2b | **完了**（2026-10-17） | timeline が stream の channel を集合で持ち，D3 の移動を `timeline::Lines`（instance の全 line）で記帳，D14 の `stream_target`（spec 07 RM-34，RM-3・RM-12・RM-13・RM-16・RM-21・RM-25・RM-26）．DeepSeek が実装（セルフレビュー各3回），Opus のレビュー（[reviews/1b2b-opus.md](reviews/1b2b-opus.md)：minor 2）と Sol 6.1 の独立レビュー（[reviews/1b2b-sol.md](reviews/1b2b-sol.md)：major 2・minor 2）．修正は3回（[1](reviews/1b2b-fix-brief.md)，[2](reviews/1b2b-fix2-brief.md)，[3](reviews/1b2b-fix3-brief.md)）．移動元が channel を手放す時刻（release）の扱いだけで Sol の指摘が5件出た（記帳時に焼き込んだ値が古くなる，移動の item 以外に効かない，`ready` と混ざる，`Stop` で切られた segment を見ない）：どれも「先に決めた値を今の値として使う」（`mech:planned-state-read-as-actual`）で，release を別の field にし，`Lines` が移動元の今の計画から一般の定義（channel を持ち移動より前に始まった segment の終わりの最大）でどの操作の後でも読み直す形に改めた．Sol の3回目の再レビュー（[reviews/1b2b-sol-rereview3.md](reviews/1b2b-sol-rereview3.md)）は新規の指摘なし，構造の継続を推奨．Provider はまだ `Lines` を使わない（1b-3・1b-4） |
