# 1b-4d-2：指示書

## plan-brief


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `273c120`，tree はきれい）．この段階は **risky**（AGENTS.md §9：thread の lifecycle，stream の identity，stop の順序を変える）．だから今回は **テスト計画だけ** を書きます．コードは書きません（Sol が計画をレビューした後に，別の指示書で実装します）．

## 1. 読むもの

- `plan/phase8/33-channels-and-streams.md`：D3（8 の「止まっている stream」），D4，D14，D15，§5 の 1b-4 行「1b-4c で 1b-4d へ送るもの」(1)〜(3)，(5)，(7)，(12) と「分け方」．
- `plan/phase8/reviews/1b4d1c-opus.md` §5 の 1 つ目（stop と owner の窓），`plan/phase8/reviews/1b4d1c-test-plan.md` の S15．
- `design/07-radio-model.md` の RM-12，RM-13，RM-16，RM-21，RM-25，RM-29．`design/18-uhd-radio.md` の §3（thread），UR-12，UR-13，UR-15，UR-16，UR-17，UR-21，UR-24，UR-25，UR-26，UR-29．
- コード（`crates/ezsdr-radio-uhd/src/provider/`）：
  - `mod.rs`：`prepare` の links（`rx_port(0)`）と `PreparedStream`（stream 0 だけ），`arm`，`start` の `[PreparedStream; 1]` と `spawn`，`stop` の join，`cleanup` の `detached`．
  - `core.rs`：`booked_stream`，`Core::new` の `rx_id`・`tx_id` と RX_OVERFLOW の handle，`links`，`:842` の `stream_node(.., 0)`．
  - `control.rs`：`Control::new`（line は stream 0 だけ），`book`（Stop，start_rx），`projections`・`channels_at`・`channel_live`・`targets`（`booked_stream(dir)`），`book_cold`（`channel_in_later_stream` の拒否，`from_none` の `k`），`book_burst`（`Addressed::Stream { k: 0 }` だけ），`:1100` の TIME_ERROR の source，`:245` の `rx_packet_samples(0)`．
  - `mode.rs`：`channel_in_later_stream`，`stream_of`，`stream_channels`，`stream_rate`，`stream_node`，`rx_port`．
  - `rx.rs`・`tx.rs`：すでに `self.k` を持つ．`rx.rs:568` の publish（全 link），`begin`（`rx.rs:277-`），`tx.rs` の `do_switch`．
- Mock の同じもの（1b-3b，commit `e4ca550`）：stream ごとの links（`lib.rs:2365-2390`），`cold_streams` の移動の拒否．

## 2. 何をするか

layout（`keys: "channel"`）の stream 1 以上を動かす．今は `Mode::channel_in_later_stream` が prepare と実行時で拒否し，`start` は方向ごとに 1 本の stream しか始めない．

## 3. orchestrator の決定

1. **範囲**：layout の stream `k ≥ 1` が，prepare・arm・start・cold 変更（`-1` から入る，`-1` へ抜ける，rate，antenna）・timed 更新・`Stop`・`start_rx`・`TxBurst` で stream 0 と同じに動く．**stream から stream への移動（RM-34）はまだ拒否する**（1b-4e）．`channel_in_later_stream` の拒否は消し，移動の拒否に置き換える（Mock 1b-3b の `cold_streams` と同じ判定：`was ≥ 0 && is ≥ 0 && was != is`）．理由の文は `UR-25:` で始める．
2. **thread**：stream ごとに 1 本．名前は両方のモードで `uhd-rx-s<k>`・`uhd-tx-s<k>`（今の key は `-s0` だけ）．layout では宣言した stream（方向ごとの channel 数，D4）の全部に thread を立て，channel の無い stream の thread は idle（UR-15 の「owner がいない stream を作らない」を stream ごとに）．join は uhd-control → TX を `k` の順 → RX を `k` の順，各 1 s（UR-16）．`rejected` の `thread` は stream の名前．`close_streams` は device 全体を閉じるので，1 本でも detach したら呼ばない（今と同じ．stream ごとの close は作らない）．
3. **link**：stream `k` の link は port `rx<k>`（`mode.rx_port(k)`）のものだけ．`Core` は stream ごとの link の列を持つ（Mock の形）．「link の無い受信 stream は streamer を持たない」は stream ごと．Block の拒否も port ごと（今の文のまま）．
4. **node・clock・event**：stream `k` の SampleClock は `<id>/rx/s<k>`・`<id>/tx/s<k>` で宣言し登録する．event の source（RX_OVERFLOW の hot path の handle を含む，D15），TIME_ERROR，LATE_COMMAND などはその stream の node．`rx_id`・`tx_id` を stream 0 に固定して使う所は，stream を持つ所では `Core::stream_node(dir, k)` にする．
5. **timed 更新**：判断に使う line（halted，planned，`targets`，live）は，その instant にその channel を持つ stream の line（`mode.stream_of`）．今の key は stream 0．command queue の depth は instance（motherboard）ごとに数え，stream ごとには数えない（UR-24 のまま）．
6. **隔離**：stream `a` の cold 変更・`Stop`・`start_rx`・拒否による halt は，stream `b` の segment を切らず，`b` の channel を configure せず，`b` を halt しない．全 stream に効くのは `<id>` 宛ての `Stop`，device の loss，`Provider::stop` だけ（RM-16，RM-21，RM-25）．
7. **stop と owner の窓**（1b-4d-1c の持ち越し）：owner（uhd-rx-s<k> の `begin`，uhd-tx-s<k> の `do_switch`）が `Planned` を取った後に `Provider::stop` が記帳されたとき，その segment の origin が stop の instant 以後なら，owner はその segment を始めない（RX は `rx_start` を出さない，TX は clock を採らない）．owner は configure の後，始める直前に lock の下で stop の記録を見直す．すでに始まった device の呼び出しは終わってよい（configure は値だけで，RF も sample も出さない，INFERRED）．理由：UHD は untimed の stream command を先に出した timed command の後で実行する（VERIFIED，1b-4d-1c の Sol の計画レビュー）ので，stop の後に出た timed start は止まらずに一度走る．
   - これ以外の仕組みは足さない（lock を device の呼び出しの間持つ，stop の新しい状態，など）．足りないと思ったら，計画の §7 に穴として書く．
8. **(12) の見直し**：mutation の P313（`equivalent`，後の stream に line が無い前提），P344・P345（owner の照合）を読み，stream 1 以上が動くと前提がどう変わるかを書き，行を直すか消すかを提案する．

## 4. 系列（各系列：テスト名，assert 1〜2 個，何で観測するか，落とす誤った実装）

| # | 系列 | 期待 |
|---|---|---|
| S1 | layout：rx c0→s0，c1→s1，port rx0・rx1 に link．prepare → arm → start(T0) | `rx_open(0,[0])`・`rx_open(1,[1])`，`rx_start(0,T0)`・`rx_start(1,T0)`；thread `uhd-rx-s0`・`uhd-rx-s1`；rx1 の block は c1 だけで，SampleClock は `<id>/rx/s1`，origin T0 |
| S2 | S1 で rx1 に link が無い | stream 1 は streamer を開かず，thread は idle；stream 0 は S1 と同じ |
| S3 | s1 に channel 無し，rx1 に link あり．走行中に `c1.stream = 1`（-1 から） | uhd-control の from-none が stream 1 で `rx_open(1,[1])`・configure・timed start；stream 0 は切れず，channel 0 に apply は無い |
| S4 | 走行中に `c1.stream` を 0 → 1（移動） | `UR-25:` の拒否．どの stream も切れず，device の呼び出しは無い |
| S5 | `c1.stream` を 1 → -1 | stream 1 の segment が cut，その thread は idle に戻る；stream 0 は不変 |
| S6 | stream 1 の rate の cold 変更（stream 0 は走行中） | stream 1 だけが cut と configure；stream 0 の segment と channel 0 は触られない（S11 の隔離） |
| S7 | `Stop` を `<id>/rx/s1` へ，後で `start_rx` を `<id>/rx/s1` へ | stream 1 だけ止まり，再開する；`<id>` 宛ての `Stop` は両方 |
| S8 | stream 1 を拒否で halt させる（configure の失敗） | stream 1 だけ halt，stream 0 は流れ続ける |
| S9 | channel 1 の timed gain（c1 は s1）．s0 は halt 中 | timed apply が channel 1 に `e` で出る（s0 の halt に止められない）；`<id>/rx/s0` 宛てでも key が channel 1 を決め，node は SpecTime の clock だけ（D14） |
| S10 | 2 stream の timed 更新で queue を埋める | depth は instance で数える（stream ごとに 2 倍にならない） |
| S11 | TX：tx c0→s0，c1→s1，`TxBurst` を `<id>/tx/s1` へ | `uhd-tx-s1` が stream 1 に送る；s0 に channel が無ければ `<id>/tx/s0` 宛ては `UR-21:` の拒否；2 stream の clock は別 |
| S12 | 2 本目の受信 stream の `delivered` の床 | stream 1 の cold 変更は stream 1 の `delivered` で床を作り，stream 0 のものを使わない（「送るもの」(5)） |
| S13 | `Provider::stop`，4 thread（rx s0/s1，tx s0/s1） | end を全 line に 1 回；join は control → tx-s0 → tx-s1 → rx-s0 → rx-s1 |
| S14 | `uhd-rx-s1` が join の 1 s で戻らない | `rejected` に `uhd-rx-s1`，他は join，`close_streams` を呼ばない |
| S15 | `uhd-rx-s1` が panic | `DEVICE_LOST`（"a Provider thread panicked"），`rejected` の thread は `uhd-rx-s1`；stream 0 の thread も loss で終わる |
| S16 | stop と owner の窓：owner が `Planned` を取る → `Provider::stop` が記帳 → owner が configure から戻る（gate で順序を固定），RX と TX の各 1 本 | RX：`rx_start` を出さない；TX：clock を採らない．stop が owner の読む前なら，device の呼び出しは無い |
| S17 | 今の key のモード | thread 名が `uhd-rx-s0`・`uhd-tx-s0` になる以外は不変（既存のテストが通る） |
| S18 | `rx_packet_samples(k)`：uhd-control の呼び出しの長さ（`control.rs:245`） | その stream の streamer の packet の長さを使う |

ハードウェアの前提（計画で出典を確かめ，VERIFIED / INFERRED を付け直す）：
- H1 X310 で，channel の重ならない 2 本の rx streamer が同時に開き，それぞれの stream command と recv を持てる．INFERRED（`multi_usrp_rfnoc.cpp` の `get_rx_stream` は streamer ごとに chain を作る；同じ port の再接続は `graph.cpp:187-197` が拒む）．bench BP0．
- H2 1 本の streamer の recv・send は 1 本の thread から，別の streamer は別の thread からでよい．INFERRED（UHD の docs と examples で確かめる）．
- H3 timed command の時刻は motherboard ごと（`set_command_time`）で，stream が何本でも queue は 1 つ．VERIFIED（spec 18 §3，UR-24）．
- H4 untimed の stream command は先の timed command の後で実行される．VERIFIED（1b-4d-1c）．

## 5. 計画に書くこと

`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d2/test-plan.md` に日本語で書く．

1. **あなた自身の系列を先に**，コードと spec から書き，それから §4 との差分（足した系列，§4 で誤っていると思うもの）．
2. 系列ごとに：テスト名，置く場所，assert（1〜2 個），観測（`FakeDevice::calls`，thread の名前，Manifest の `rejected`，SampleClock の記録，event の source，plan の cut/origin），落とす誤った実装．
3. 既存のテストのうち，thread 名や stream 0 の固定に触れて直すものの一覧（名前を変えるなら `renames.txt` の行）．
4. 変える `design/` の文：spec 18 の §3（thread），UR-12（stream 1 以上の拒否），UR-15，UR-16，UR-17，UR-21，UR-24，UR-25（移動の拒否），UR-26，§6 の表と Changes．DA-1 の形で．他に偽になる文を grep で探して挙げる（`stream 0`，`uhd-rx`，`uhd-tx`，`later stream`）．
5. mutation の行の方針：新しい規則ごとに 1 行（対応だけ．文字列はコードと一緒に書く）．§3 の 8 の P313・P344・P345 の扱い．
6. 大きさ：本体とテストの行数の見積もり（1,500 行前後．大きく超えるなら分け方を提案する）．
7. 確かめられなかったこと（INFERRED）と，§3 の決定で穴と思うもの．

## 6. 守ること

- **コードを書かない**．tree を変えない．`cargo fmt`・`rustfmt`・`mutate.py`・`remote_mutate.py` を流さない．commit・push・`git add` をしない．
- 読むための `cargo test` は流してよい（build の出力は tree の外の，いつもの target）．**1 本のテストや 1 つの test binary には `timeout 300` を付ける**．背景の job を残さない．長い command は前景で流す．
- Python は `PYTHONDONTWRITEBYTECODE=1`．
- **§3 に無い仕組みを足さない**．足りないと思ったら，足さずに §7 に穴として書く．
- 前の段階で出た誤り（同じ形を繰り返さない）：到達できる系列を「到達できない」と言って消す．実際の経路を通さず，記録を手で書き換えて確かめる．していないことを書く．
- 書けたら，最初の行を `STATUS: done`（または `STATUS: blocked: <理由>`）にして終える．**起きたことを書く．していないことは書かない**．

## impl-brief


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `273c120`，tree はきれい）．この段階は risky（AGENTS.md §9）．

## 読むもの（`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d2/`）

優先の順：
1. `adjudication.md`（orchestrator の裁定．計画と食い違えばこちら）
2. `sol-plan-report.md`（Sol の計画レビュー．裁定が採った項目の中身）
3. `test-plan.md`（あなたが書いた計画）
4. `plan-brief.md`（§3 の決定．ただし §3.4 の LATE_COMMAND と §3.7 の再確認は裁定で取り下げた）

## やること

1. **計画を v2 にする**：`test-plan.md` を裁定のとおりに直し，`test-plan-v2.md` として保存する（S11・S12・S13・S16 の書き直し，TX の隔離 4 系列，configure 中の loss，start の途中の失敗，off・空の on・halt，mutation の対応表の訂正，H2・H3，見積もり）．
2. **実装する**：計画 v2 のとおり．計画に無い状態や失敗の経路を実装で足したら，テストを足して計画 v2 と報告に書く．
3. **`design/18-uhd-radio.md`**：計画 §4 に裁定 2.7 を加えた形で，DA-1 で書く．
4. **mutation の行**：新しい規則ごとに 1 行（P436 以降）．P280・P308・P309 を消し，P313 を当て直す．動いた既存の行は当て直す．名前を変えたテストや識別子を引く行は，`new` に古い識別子が残っていないか grep する．
5. **rules.py**：消した・名前を変えたテストの引用は `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d2/renames.txt` に書いて `--renames` で渡す．

## 守ること

- **裁定と計画に無い仕組みを足さない**．足りないと思ったら，足さずに報告の「穴」に書く（1b-4d-1c の修正の回 2 で，自分で足した仕組みに順序の穴があった）．
- `cargo fmt`・`rustfmt` を流さない（tree はわざと rustfmt の形にしていない．mutation の行はソースの文字列にちょうど一致する必要がある）．新しいコードは周りの書き方に手で合わせる．
- mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけで流す．`mutate.py` をローカルで流さない．main tree に mutant を当てない．
- commit・push・`git add` をしない．
- `plan/*/tools/mutations.json` はファイル全体を書き直さない．変える行・足す行だけを，周りの書式のまま書く．
- **1 本のテストや 1 つの test binary を流すときは `timeout 300` を付ける**．背景の job を残さない．長い command（`cargo test --workspace`，`rules.py`，`remote_mutate.py`）は前景で，timeout を延ばして（600000 ms）流す．
- Python は `PYTHONDONTWRITEBYTECODE=1`．
- テストの繰り返しは helper にする（RX・TX 共通）．テストは系列ごとに assert を 1〜2 個．sleep で競合を作らない（gate と probe で同期する）．gate は失敗時にも離す．
- 前の段階で出た誤り（同じ形を繰り返さない）：計画の要のテストを飛ばす．到達できる系列を「到達できない」と言って消す．していない実装を報告に書く．記録を手で書き換えて，実際の経路を通さずに確かめる．

## 確かめること

1. `cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．（`--features uhd` の clippy はこの機械では流せない．orchestrator が image の中で流す．）
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d2/renames.txt`．
3. mutation：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 <IDs>` で，足した行・変えた行と，コードかテストが変わった行を流す．全リストは orchestrator の gate で流す．COMPILE ERROR も生き残りとして扱う．

## セルフレビュー

reviewer は DeepSeek．回ごとに新しく起動する．

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat <チェックリストのファイル>)" > <ログ> 2>&1
```

チェックリスト（ファイルに書く）．読む範囲は `git diff HEAD` のうち `crates/ezsdr-radio-uhd/` と `design/18-uhd-radio.md`．reviewer には「read-only．cargo・mutation・mutate.py・remote_mutate.py を流さない．指摘を P1〜P3 で，`path:line` つきで．無ければ『P1 なし』」と書く．項目：
1. 計画 v2 の各系列にテストがあり，そのテストが計画の言うものを観測し，mutation の行がそれを壊すか．
2. 範囲：計画 v2 に対して足りないもの，余計なもの（裁定に無い仕組み）．
3. 「全 stream」「stream ごと」の主張の grep：`booked_stream`，`stream_key(Dir::.., 0)`，`rx_port(0)`，`stream_node(.., 0)`，`rx_packet_samples(0)`，`"uhd-rx"`，`"uhd-tx"`，`rx_id`，`tx_id` が production code に残っていれば，それが stream 0 だけで正しい理由．
4. この変更で偽になる `design/18` の文（`halts the direction`，`stream 0`，`uhd-rx`，`uhd-tx`）が残っていないか．

P1 が無くなるか 3 回まで．判定を出さずに終わった回は 1 回だけ起動し直す．指摘はすべて直すか，理由をつけて退ける．

## 報告

`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d2/report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．中身：直しと `path:line`，足した・直した・消したテストと行，確かめることの結果（mutation は行ごと），セルフレビューの指摘と扱い，本体とテストの実際の行数，穴．**起きたこと（固まったテスト，途中の失敗）も書く．していないことは書かない**．

## fix1-brief


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `273c120`，tree に 1b-4d-2 が未 commit で入っている）．Sol のコードレビュー（`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d2/sol-report.md`，判定 FAIL）の 4 件を，次のとおり直してください．計画は `test-plan-v2.md`，裁定は `adjudication.md`，前の報告は `report.md`．

## 1. Sol major 1：start の途中の device loss（採る）

- `start`（`provider/mod.rs:683-718`）で，`rx_start` のエラーが `lost` なら，その loss を公開してから（`Core::device_lost`，または既存の `device_failed` の loss の扱いと同じ形）開始済みの owner を止める．`DeviceError.lost` を文字列にして捨てない．
- 次の stream の `rx_start` の前に `core.is_lost()` を見る．公開済みの loss があれば，その stream を始めずに失敗の経路へ行く．
- 拒否の理由の文は今の `UR-15:` の形に合わせる．loss の後に device を呼ばない（`rx_stop`，`close_streams` の扱いは既存の loss の経路と同じ：`mark_lost` が先）．
- テスト：(a) s1 の `rx_start` が `lost: true` で失敗する → `DEVICE_LOST` が 1 件，`start` は `Rejected`，loss の後に `rx_stop` が無い（`mark_lost` が `close_streams` より前，既存の `assert_marked_lost_before_closed` の形）．(b) s0 の owner が loss を公開した後に s1 の start が来る交差 → s1 の `rx_start` は呼ばれない（gate か probe で順序を固定）．
- mutation：loss の公開を消す行，`is_lost()` の検査を消す行，各 1 行．

## 2. Sol major 2：S16 の観測（採る）

裁定 §2.1 の期待はそのまま（再確認は足さない）．観測だけを直す．
- **RX**：e2e で決められないなら，`rx.rs` の unit で同期点を作る（`test_support` の `HoldApply` などの既存の道具を使う）：owner が `Planned` を取り，configure の戻りで止まる → 別の側で `streams.stop` と End を記帳 → gate を離す → owner の turn を回す．assert：(1) owner は timed start を出し（今の挙動），block は 1 つも配られず，その segment の cut は origin で，origin の後に untimed の `rx_stop` が出る．(2) owner の loop が終わる（thread なら join の 1 s の中，unit なら turn が `Exit` を返す）．detach の経路を期待にしない．e2e の今のテストは，detach を許す形のまま残すか消すかを判断し，報告に書く．
- **TX**：`ur_26_a_transmit_switch_at_a_stop_keeps_its_clock_and_sends_nothing` で，採った clock の上の burst を owner に渡し，gate を離した後の通常の owner の処理まで進めて，`tx_send` が出ないことを確かめる．
- mutation：TX で「終わった clock の burst を送る」形の行を 1 行（例：segment の end を見ずに送る）．RX の新しいテストを壊す行を 1 行．

## 3. Sol minor 3：gate の解放（採る）

- S13 の解放 guard を `entered_n()` の待ちの前に置く．S14 も guard にする（`Drop` で離す）．T4 は assert の失敗でも `reading` を下ろす guard にする．
- detach の後の sleep を，worker の終わりの確認（終了の通知を待つ）に替える．sleep で終わりを代用しない．

## 4. Sol minor 4：UR-23 の Stop の宛先（採る）

`design/18-uhd-radio.md:245`：burst を終える宛先に，current keys の `<id>/tx` を戻す（`<id>/tx`，layout では `<id>/tx/s<k>`，または `<id>`）．

## 守ること・確かめること・報告

- `impl-brief.md` の「守ること」と同じ（**裁定に無い仕組みを足さない**，`cargo fmt` を流さない，mutation は `remote_mutate.py` だけ，`mutations.json` は全体を書き直さない，commit・push・`git add` をしない，1 本のテストには `timeout 300`）．
- 確かめること：`cargo test --workspace --no-fail-fast`，clippy，`rules.py --base HEAD --renames ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d2/renames.txt`，mutation は新しい行と触った行（触ったテストを殺す既存の行も）．
- セルフレビューは 1 回（DeepSeek，read-only，`impl-brief.md` の形）．チェックリスト：(1) `start` の全ての失敗の経路で，loss が公開され，loss の後に device を呼ばないか，(2) S16 の RX・TX のテストが裁定 §2.1 の期待を実際の経路で観測し，detach を期待にしていないか，(3) gate と guard が失敗時にも離れ，sleep で終わりを代用していないか．
- 報告：`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d2/fix1-report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．**起きたことも書く．していないことは書かない**．
