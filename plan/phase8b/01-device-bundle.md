# 8b step 0，問い ④：一つの radio が N 台の X3xx をまたぐときの UHD Module の形（決定した設計）

| 欄 | 値 |
|---|---|
| 状態 | **owner が 2026-11-20 に決めた**（D-1〜D-6，Fable の決定 1〜3．[01-device-bundle-decisions.md](01-device-bundle-decisions.md)）．Fable 5.2 の review（[reviews/01-device-bundle-fable.md](reviews/01-device-bundle-fable.md)）の major M1〜M4 を直し，minor m0〜m7 を §11 で扱った版．書いたのは Opus（effort max）．読むだけの作業で，木は変えていない |
| 決定の一覧 | **D-1** 束は一要素一台の順序つきの list で，合成の `Bundle` の裏に隠す（B）．compound の要素（G）は後で．同期していない radio は別の Run．**D-2** どの装置の喪失も radio の喪失．**D-3** stream は装置をまたがない．**D-4** 分配器か daisy chain，装置ごとの source の list，N ≥ 2 は一つの PPS の縁で時刻を入れ，epoch は `pps`．**D-5** Manifest は装置ごとの行で装置を名指す．**D-6** 止まった stop の join は段ごとに共有する 1 s．**F1** 2026-11-27 の後に確認済みとするのは「秒が合う」までで，装置の間の tick の揃えは S6 まで INFERRED．**F2** どの装置の喪失でも全 handle に印を付け，process の終わりまで保持する．**F3** 時刻合わせは確認が外れたら 3 回までやり直し，確認は handle 0 の二回の読みで挟む．N = 1 も同じ道を通る．orchestrator の決定は §10 |
| 根拠 | main `e008038b`（`git show main:<path>`）．行番号はこの commit のもの．作業中に main は `ea7bc5c1`（#117）に進み，device gate の下で受け付ける受信の呼び出しが六つになった（UR-29）．Fable は引いた範囲を `ea7bc5c1` で確かめ，行は合っていた（m7）．UHD は v4.10.0.0 を `gh api repos/EttusResearch/uhd/contents/<path>?ref=v4.10.0.0` で読んだ．v3 は証拠としてだけ使う（AGENTS §5） |
| 範囲 | ④ の形（スレッド，事実の持ち主，device gate，stop と cleanup，参照と時刻の手順）と，それが決める利用者の契約．selector の細部，MockRadio の複数台，coherence の宣言は §9 の後の段に回す |

## 1. 決まった形（要約）

- **Provider の形は今のまま**：uhd-control が 1 本，宣言した stream ごとに `uhd-rx-s<k>` と `uhd-tx-s<k>`，一つの device gate，stream が単位の開始と巻き戻し．
- **束（D-1）**：binding の `args` は装置ごとの UHD の device string の list で，参照の源を先頭に置く．どの装置も `internal` のまま list の順に開く．合成の `Bundle` が今の `Device` を実装し，channel と stream を handle に写す．装置ごとの事実は `Bundle` の method で持つ．N = 1 も一要素の束で，同じ道を通る．
- **参照と時刻**：全部を開いた後で，Authority が源を先にして source を切り替える．源の REF OUT と PPS OUT は UHD が開くときに ON にする（M1 で直した）．続いて Module が一つの PPS の縁で全 handle の時刻を入れ，handle 0 で挟んで確かめる．外れたら 3 回までやり直し，それでも外れたら Session を拒む（F3）．確かめられるのは秒の一致までで，装置の間の tick の差は配線で決まり，S6 で測るまで INFERRED（F1）．
- **足すもの**：装置ごとの事実（§3.1），段ごとに共有する join の上限（D-6），一回に一台の点検（§3.6）．
- **喪失（D-2，F2）**：どの装置の喪失でも，`DEVICE_LOST` は一つで全 stream が終わる．全 handle に印を付けて process の終わりまで保持し，以後どの装置も呼ばない．message は分かれば装置を名指し，分からなければ `unknown`．
- **同期していない radio** は別の Session（別の Run）で使う．Kernel は変えない（§12）．

## 2. 事実

### 2.1 今の Module（N = 1）— VERIFIED（main）

- **スレッド**：uhd-control，受信 stream ごとの `uhd-rx-s<k>`，送信 stream ごとの `uhd-tx-s<k>`（channel を持たない stream にもある），Authority の `uhd-clock`（`design/18-uhd-radio.md:44-59`，`provider/mod.rs:666-712`）．
- **uhd-control は booking で装置を呼ばない**．装置を呼ぶのは 500 ms ごとの点検だけ（`provider/control.rs:645-674` の `ref_locked` と `time_now`．`:234-644` に `device.` の呼び出しは無い）．
- **喪失は instance に一つ**：`Core.lost`（`core.rs:826`）と `device_gate`（`core.rs:792`）．`device_lost(&self, message)` は装置の識別を取らない（`core.rs:1076-1098`）．呼ぶ所は 10 か所（Fable M3：`control.rs:654, 665, 670`，`rx.rs:594, 614`，`tx.rs:1383`，`core.rs:1104, 1283, 1368`，`mod.rs:409`）．`Device::mark_lost(&self)` も引数を持たない．喪失の後は誰も装置を呼ばず（`tx.rs:246-249`），`cleanup` は `close_streams` を飛ばし（`mod.rs:778`），失った装置は解放しない（UR-29，`uhd.rs:671-697`）．
- **止め方**：`Provider::stop` は終わりを記帳し，uhd-control を join し，全送信に Shutdown を送って一本ずつ join し，全受信にも同じことをする（`mod.rs:722-764`）．join は一本ごとに新しい 1 s（`JOIN_BOUND`，`mod.rs:74, 419-439`）．`stop` 自身は装置を呼ばない．送信の最後の `BURST_ACK` を待つ 100 ms は，各 uhd-tx の shutdown の中にある（`tx.rs:1481-1502`）．
- **`uhd.rs` は装置ごとの呼び出しを mboard 0 に固定している**（`uhd.rs:794, 796, 843-844, 855, 864, 874, 884`，`answers` は `:637-642`）．全 mboard に効くのは `set_time_unknown_pps`（`:853`）だけ．
- **RS-8a**：締め切りは cleanup の step ごと・fragment ごとに 5 s（`crates/ezsdr-kernel/src/run.rs:518-525, 556-571`）．Provider の締め切りの hook（`CleanupOps::deadline_millis`）は coordinator の `Ops` が上書きしていない（`ending.rs:297`）．放棄された step の呼び出しは slot を握ったまま走り続け，step 4 は `KC-39` で `cleanup()` を呼ばない（`ending.rs:341-352`）．組み立てのときもまだ握られていれば，`KC-44` でその Provider の sections と fidelity が Manifest から落ちる（`ending.rs:53-72`）．
- **一つの Run に UHD の binding は一つ**：EA-7（`design/16-easy-api.md:43`）と UR-6（`design/18-uhd-radio.md:159`）．

### 2.2 UHD v4.10.0.0 — VERIFIED（source）

- **開き方**：compound の mboard は並列に初期化され，`serialize_init` のときだけ順になる（`host/lib/usrp/x300/x300_impl.cpp:193-226`，`x300_defaults.hpp:50`）．一台ずつ開くときは，C API の `uhd_usrp_make` が一つの mutex の下で直列に走る（Fable m6：`usrp_c.cpp:215-218`）．
- **開くときの参照と出力（M1 で直した）**：
  - 各 mboard の controller は，構築の最初に clock の制御 register を既定値で書く．参照も PPS も `internal`，`PPS_OUT_EN = 0` である（`x300_regs.hpp:217-225`，`x300_mb_controller.cpp:72`，`soft_register.hpp:173-181`）．
  - 続いて device args の `clock_source` と `time_source` を設定する（`x300_mb_controller.cpp:75-76`．既定は `internal`，`x300_defaults.hpp:39-40`）．初期化の中では lock を 1 s しか待たず，lock しなくても報告しない（`:236-262`）．FPGA の radio clock の PLL は 10 ms で lock しなければ例外になる（`:266-275`）．
  - その後で `x300_impl::setup_mb` が `init_prop_tree` を呼び（`x300_impl.cpp:374`），**PPS OUT を ON にする**．`time_source/output` を作って `set(true)` し，その subscriber が `set_time_source_out(true)` を呼ぶ（`x300_prop_tree.cpp:64-68`，"setup the time output, default to ON"）．REF OUT は LMK の初期化で出る（`x300_clock_ctrl.cpp:925`．`clock_source/output` の property は初期値を書かない，`x300_prop_tree.cpp:98-101`）．
  - **帰結**：開き終えた X300 は REF OUT も PPS OUT も出す．切れるのは開いている途中だけで，register の書き込みから `init_prop_tree` まで，LMK の初期化と 1 s の lock 待ちを含む数秒である（PPS OUT は VERIFIED，REF OUT の途切れは INFERRED）．
- **出力と PPS の中身**：PPS OUT は「選んだ PPS」と `PPS_OUT_EN` の AND である（FPGA `fpga/usrp3/top/x300/x300.v:478-494`）．内部の PPS は参照の clock（`ref_clk`）を割って作る（`x300.v:468-477`）．PPS はまず参照の clock（10 MHz）で取り込まれ，次に radio clock へ移る（`fpga/usrp3/lib/timing/pps_synchronizer.v` の冒頭の注釈）．電源を入れた直後の FPGA は PPS OUT を出さない（Fable：`bus_int.v:386-391`）．
- **時刻**：`set_time_unknown_pps` は board 0 の縁を最大 1.1 s 待ち，`set_time_next_pps(ALL_MBOARDS)` を出し，1 s 後に各 board の時刻を比べるが，外れていても警告するだけ（`multi_usrp_rfnoc.cpp:909-946`）．C API には，装置ごとの `get_time_last_pps`・`set_time_next_pps`・`get_time_now`・`set_clock_source`・`set_time_source`・`set_clock_source_out`・`set_time_source_out`・`get_mboard_sensor` がある（`host/include/uhd/usrp/usrp.h:350-448`）．
- **control の往復**：X300 の control は UDP の一往復で，応答が無ければ 1.0 s 待って最大 3 回やり直す（`x300_fw_ctrl.cpp:29, 44-57, 124, 159`）．
- **claim**：X300 は process の hash で claim する．別の process が持っている装置を開くと `Failed to claim device` になる（`x300_impl.cpp:254-256`，`x300_claim.cpp:26-51, 60-76`）．
- **compound**：X3x0 は compound にでき，compound の中の複数 channel の streamer は UHD が揃える（`host/docs/driver_usage/multiple.dox`，`host/examples/rx_multi_samples.cpp:41-44, 151-158, 195-200`）．この設計では使わない（§5）．

### 2.3 v3 と bench — VERIFIED

- **v3**：複数台は二通りに書けた．一つは `addr0=…,addr1=…` の一つの `multi_usrp`（`v3/docs/v3/usrp.md:12-65`），もう一つは device ごとに別の `multi_usrp` である．v3.0.17 は「PPS信号を出力する端末から順番に起動しないと起動に成功しない問題」のために devices の初期化の順を指定できるようにした（`v3/changelog/v3.0.20.md`．見出しは v3.0.17）．各 device は `make` の後に mboard ごとに source を設定した（`v3/cpp/uhd_usrp/multiusrp.cpp:307-345`）．`source_out` は一度も呼ばない（`grep -rn source_out v3/cpp v3/source` は 0 件）．UHD は 4.7（`v3/docker/v3_prebuild/Dockerfile:1`）．M1 により，v3 の失敗は「源がまだ開いていない（出力していない）のに，受け手を `external` で開いた」ことと読める．
- **bench**：#1 X300＋OBX（192.168.40.36，enp1s0f1np1），#2 X300＋UBX（slot A）・CBX（slot B）（192.168.47.37，enp1s0f0np0）．PPS と 10 MHz は #1 から #2 への daisy chain．`handoff.md:23` に「#1 の `ref_locked` は false（電源を入れ直しても変わらない）」とある．#2 の `test_pps_input` は成功した（`plan/phase8/bench-results.md:274`）．

### 2.4 今の stop の最悪（INFERRED：コードからの算術）

`Provider::stop` の最悪は 1 + n_tx + n_rx 秒になる．KC-46b の「約 3 s」（`design/06-kernel-coordinator.md:446`）は，一方向に stream が一つのときだけ正しい．

**反例**：X300 2 台，各 UBX 2 枚．両方がつながる switch が stop の最中に落ち，8 本と uhd-control が UHD の中で止まる．join は 1 + 4 + 4 = 9 s 待ち，RS-8a が step 2 を 5 s で放棄する．step 4 は `cleanup()` を呼ばず（KC-39），`timing`，`bursts`，`async`，`rejected`，`stats` が Manifest に入らない．組み立てのときもまだ握られていれば，すべてが落ちる（KC-44）．

## 3. 形

### 3.1 束と装置の境界（D-1，m0，m2，m4）

- **binding**：`args` は UHD の device string の list で，一要素が一台．参照の源を先頭に置き，要素の順が timekeeper の番号になる．RM-28 の timekeeper ごとの list と同じ形で，N = 1 も一要素の list にする（今の文字列は list の形を示して拒む）．要素の中に `clock_source` と `time_source` は書けない（拒む）．source は selector の timekeeper ごとの list で与える（D-4）．
- **開く前の照合（m4）**：要素ごとに `uhd_usrp_find` で serial を引き，二つの要素が同じ装置を指すとき（X310 の二つの SFP の address など）は，どれも開かないで拒む（find の結果に `serial` が入ることは INFERRED）．UR-5 の serial の登録は開いた後のまま残す．
- **`Bundle`（m0 の (b)）**：合成は具体の型 `Bundle` とし，`from_binding` と Authority と `Core` に**具体の型で**渡す．
  - `Bundle` は今の `Device` を実装し，Provider の hot path の呼び出し（§2.1 の約 66 か所）は変わらない．
  - channel の method（`front_end`，`rates`，`apply`）は，instance の channel を（handle，その中の channel）に写す．
  - stream の method（`rx_*`，`tx_*`，`rx_packet_samples`）は stream の handle に渡す．`rx_open` と `tx_open` は，stream の channel が一つの handle の上にあることを確かめる（D-3）．`TxReport.channel` は streamer の中の位置なので変わらない．
  - 装置ごとの事実（`serial(i)`，`set_sources(i, …)`，`time_now(i)`，`ref_locked(i)`，`time_last_pps(i)`，`set_time_next_pps(i, …)`）は `Bundle` の method とし，trait には番号を漏らさない．
- **`Device` の単数の method の意味（m2）**：
  - `master_clock_rate`：全 handle で等しくなければ `from_binding` が拒む（UR-5）．tick の変換は各 handle の値を使う（`uhd.rs:597`）．
  - `time_now`：handle 0．`ref_locked`：Provider は `Bundle` の装置ごとの method を使う．trait の `ref_locked` は全 handle の AND で，使う所は無い．
  - `describe`：handle ごとの list．`fidelity`：いちばん弱いもの．
  - `host`：全 handle が同じ時計を返す．実機は実時間，fake は共有の手動の時計．違えば `Bundle` の組み立てが拒む．
  - `mark_lost`：全 handle に送る（F2）．`close_streams`：全 handle に送る．
- **葉の `Device` の trait の変更（UR-2）**：`time_last_pps()` と `set_time_next_pps(ticks)` の二つを足す．どちらも「この装置には無い」という `DeviceError` を返す既定の本体を持たせ，今の `impl Device` の wrapper はそのまま通す．`UhdDevice` は C API の `uhd_usrp_get_time_last_pps` と `uhd_usrp_set_time_next_pps`（`usrp.h:357, 371`）で実装し，`FakeDevice` も実装する．REF OUT と PPS OUT を出す呼び出しは足さない（UHD が開くたびに ON にする，§2.2．§11 の M1）．
- **N = 1**：一要素の `Bundle` を通る（M4：道を一つにする）．N = 1 の振る舞いが変わらないことは，`Bundle` を通した N = 1 を今の differential の rig に載せて確かめる．

### 3.2 事実の持ち主（今 → N 台）

| 事実 | 今の持ち主（main） | N 台 |
|---|---|---|
| 装置の handle と teardown | `UhdDevice`，`mark_lost`，kept の list（`uhd.rs:621-697`） | handle ごとに今の `UhdDevice`．`Bundle` が束ねる．喪失では全 handle に印を付けて保持する（F2） |
| 喪失（`lost`，瞬間，全 line への終わりの記帳） | `Core`，`device_gate` の下（`core.rs:826, 1076-1098`） | 同じ．instance に一つ（D-2） |
| 喪失を受け付ける gate | `device_gate`（`core.rs:956-965, 1553-1567`） | 同じ．gate は一つ |
| stream の計画，holder，読みの印，装置の記録，送信の book | `Streams`，line の lock | 同じ．`k` は instance の通し番号 |
| 受信の装置の状態，送信の装置の chain | `RxChain`（L1），book の `DeviceChain` | 同じ |
| rate の照合の結果 | `Core.rates[dir]`（`core.rs:823`） | 装置と方向ごと |
| serial の登録 | `UhdRadio._serial`（`mod.rs:192`） | 装置ごとに一つの `SerialGuard` |
| 参照の lock，時刻の読みの失敗の起点 | `Control.clock_lost`，`failing_since`（`control.rs:147-150`） | 装置ごと．成功した読みは**その装置の**数え直しだけをする |
| root の時刻（anchor） | `DeviceAuthority` と `uhd-clock` | handle 0 を読む |
| 参照の切り替えと時刻合わせ | `DeviceAuthority::new`（`authority.rs:351-361`） | 同じ所で，§3.7 の手順．N によらず一つ |
| スレッドの生成，join，detached | `UhdRadio`（`mod.rs:394-439, 722-785`） | 同じ．join は段ごと（D-6） |
| host の時計 | `device.host()`（`core.rs:885`） | instance に一つ（`Bundle` が全 handle で同じことを確かめる） |

### 3.3 スレッドと device gate（問い 1，m1）

- **スレッドは stream ごと**（今のまま）．N 台では stream の数だけ増える．stream は一つの装置の上にある（D-3）ので，持ち主のスレッドが一本，その streamer が一つになる．
- **uhd-control は一本**．これは Kernel の事実で，`ActionReceiver` も KC-21a の待ちも instance に一つである（MA-14，MA-14b）．
- **device gate は一つ**．喪失が instance に一つなので，gate が受け付ける呼び出し（streamer の開閉，rate の読み，UR-29 の六つの受信の呼び出し）は全装置で同じ gate を取る．
  - gate は一回の装置の呼び出しの間だけ握られるが，死にかけの装置では一回が 1〜3 s 塞がる（§2.2 の 1.0 s × 3．stream command の道の timeout は読んでいない）．
  - その間，健全な装置の持ち主も gate を待つ．結果として D-6 の段の 1 s を越え，健全な装置のスレッドまで detached になりうる（KL に書く，§10）．D-2 のもとでは結果（radio の喪失）は同じなので，gate を装置ごとにはしない．

### 3.4 喪失（D-2，F2，問い 1）

- どの装置の喪失も instance の喪失になる．喪失の形は，その装置の呼び出しの `lost: true`，その装置の時刻の読みの 1 s の失敗，受信の 1 s の沈黙，スレッドの panic である．`DEVICE_LOST` は一つ（MA-9a，source は instance の root）で，全装置の全 stream が喪失の瞬間に終わる．以後どの装置も呼ばない（UR-29 の規則のまま）．
- **全 handle に印**（F2）：`Bundle::mark_lost` は全 handle に送り，どれも process の終わりまで保持する．`mark_lost` に装置の識別は要らない．健全とみなした装置を解放する窓（Fable M3 の abort の列）も無い．server は Session ごとに一 process なので，利用者に見える違いは無い．
- **名指し**（orchestrator が Fable の item 4 を採った）：装置が分かるときは，message の頭と `timing { device_lost }` の行に装置を書く．分からないときは `unknown` と書く．
  - 装置を渡すように変える所は四つ：点検の `ref_locked` と時刻の読みの三つ（`control.rs:654, 665, 670`．どの装置を読んだか分かっている）と，受信の沈黙（`rx.rs:614`．stream の装置）．
  - 装置の呼び出しが `lost: true` を返した所（`rx.rs:594`，`tx.rs:1383`，`core.rs:1104, 1283, 1368`）は変えない．`Bundle` が最後に `lost: true` を返した handle を覚えていて，それを使う．
  - panic（`mod.rs:409`）は `unknown` になる．
- **健全な装置に起きること**（INFERRED）：送信は渡した分（in-flight window `W` = 10 ms 以内）を出して underflow で黙る．Stop の後の 10〜12 ms と同じ程度である（KL-13）．受信は読む者がいないので overflow で止まる（再開は読み手が overflow を読んだときにだけ出る，spec 18 §2．radio が自分で止まることは Fable が `radio_rx_core.v:586-595` で読んだ．flow control が radio まで背圧を返すことは INFERRED のまま）．
- `continue` の Policy では，N = 1 と同じく動く stream の無い radio のまま続く．
- **閉じない約束**（D-2）：装置ごとの喪失を radio の致命的でない event として足す道は残す（Module と語彙の変更で，Kernel は変えない，INFERRED）．
- **8c の注意**（Fable §4）：Replay のように装置の中に源を持つ送信は，「以後どの装置も呼ばない」では健全な装置で止まらない．それを足す段は，喪失の公表の前に健全な handle へ stop を出す道を一緒に足す．

### 3.5 Stop と cleanup（D-6，問い 2）

- N 台でも instance は一つで，`Provider::stop` も RS-8a の 5 s も一つである．
- 順序は instance 全体で段に分ける．uhd-control，全装置の送信，全装置の受信の順で，今と同じく RM-16 と RS-6 の「送信を受信より先」を守る．段の中のスレッドは同時に動くので，装置をまたいで並列に止まる．
- **上限は段ごとに共有する 1 s**（D-6）．最悪は 3 s ＋ polling で，N にも stream の数にもよらない．健全な stop は 0.2〜0.4 s 程度（INFERRED．`timing` の `stop { at, done }`，`mod.rs:762` で測れる）．
- 段の 1 s までに終わらないスレッドは detached として `rejected` に名前を残す．`cleanup` は `close_streams` をせず，leak を記録する（UR-16 のまま）．slot は step 4 に空き，sections は Manifest に入る．負債 #20 はこれで閉じる．KC-46b の「約 3 s」も，この後はどの layout でも正しくなる．
- cleanup の形は今のまま．streamer の数だけ解放に時間がかかる（INFERRED．Manifest の `Stopping` から `CleanedUp` までの時刻で測る）．

### 3.6 KC-21a と `min_command_lead`（問い 3）

- booking は装置を呼ばない．`<id>` への `Stop` も全 line への記帳で終わる．だから KC-21a の待ちは N によらない．
- **点検は一回に一台**：500/N ms ごとに一台の `ref_locked` と `time_now` を順に読む．各装置は今と同じく 500 ms ごとに読まれ，1 s の失敗は装置ごとに数える．一回の点検で uhd-control が装置に使う時間は N = 1 と同じになる．
- `min_command_lead` は instance に一つで，語彙の 5 ms のまま．control の mutex は handle ごとにあるので，装置の違う configure は互いを待たない．ただし UR-29 の六つの受信の呼び出しは gate で一列になる（§3.3）．同時に始める stream が多いと，1 channel で測った 10 ms の受信の start lead を越えうる．越えれば `START_LEAD_SHORT` か遅れた開始として報告される（UR-25）．値は bench の後に owner が決める（KL-15）．
- Authority の `uhd-clock` の anchor の読み（handle 0）は，handle 0 の mutex だけを取る．

### 3.7 参照の切り替えと時刻合わせ（D-4，F1，F3，M1，M4，m3）

- **順**：server は今のまま `from_binding` の後に `DeviceAuthority::new` を呼ぶ（`crates/ezsdr-server/src/catalogue.rs:147, 155`）．
  1. 開く：どの装置も `internal` で，list の順に開く（§3.1）．初期化はほかの装置に頼らない．
  2. 切り替える：Authority が list の順に切り替え，timekeeper 0 を先にする．2 台目以降は `external` に切り替えて lock を待つ（最大 30 s，`x300_mb_controller.cpp:236-255`）．
     - daisy chain では，timekeeper 0 が源で，自分の参照（selector の値，ふつう `internal`）のまま REF OUT と PPS OUT を出す．出力は開いたときから出ている（§2.2）．
     - 分配器では，timekeeper 0 も `external` に切り替える．
- **束の source の規則**（D-4）：`clock_source` と `time_source` は装置ごとの list で与える．
  - 2 台目以降が `external` でない束（`internal` も `gpsdo` も）は，`from_binding` が開く前に拒む．理由は「同期していない装置は別の Session（別の Run）で使う」と案内する．
  - **限り**：この拒否は宣言した source を見るだけで，配線は確かめられない．時刻の確認（下の 4）は，PPS の届かない装置や別の PPS の装置を捕まえる．時刻が 10 ms 以上ずれるからで，縁がたまたま 10 ms 以内に重ならない限り捕まる（INFERRED）．しかし別の 10 MHz に lock した装置は捕まえない（その装置の `ref_locked` は true になる）．それを捕まえうるのは，下の item 6 の増分の確認だけである．
  3. 時刻を入れる：下の手順で入れる．U5 と UR-7 の順（source が先，時刻が後）のまま．
- **避けている危険**（M1）：危険なのは，受け手が lock した後で源を開く（開き直す）ことで，源の出力が開いている間だけ途切れる．v3.0.17 の「源を先に」はこれを避けるための変更と読める．B は全部を `internal` で開いてから切り替えるので，この危険を通らない．
  - `reopen_on_unlock`（m3）は束ごと開き直す．全 handle を捨て，全部を `internal` で開き直す（`catalogue.rs:133-165` の loop は radio と device を捨てて開き直す）．一部だけ開き直すことはしない．
  - 別々の Run が daisy chain で参照を分け合う構成（m5）では，源の Run を開く・開き直すたびに他方の参照が途切れる．KL に書く（§10）．
- **時刻を入れる手順**（F3，M4．N = 1 にも同じ一つの道．外部の PPS で時刻を入れるときは必ずこれを使い，`uhd_usrp_set_time_unknown_pps` は使わない）：
  1. handle 0 の `time_last_pps` が変わる（縁）まで待つ．host の時計で約 1 ms ずつ眠り，最大 1.1 s．縁が来なければ「handle 0 に PPS が来ない」としてこの試行を失敗にする．
  2. その秒のうちに，全 handle に `set_time_next_pps(0)` を出す．
  3. handle 0 の次の縁の後まで待つ．
  4. 確かめる．`a = last_pps(h0)`，`L[h] = last_pps(h)`（h ≥ 1），`b = last_pps(h0)` の順に読む．`a ≠ b` なら縁をまたいだので読み直す（3 回まで）．次の三つがそろえば通す．
     - `a` が 0 か `mcr` の整数倍である（時刻が縁で 0 に入った．N = 1 でもこの確認が効く）．
     - `L[h]` がすべて `a` に等しい．
     - `time_now(h)` を同じく handle 0 の二回で挟み，10 ms 以内にある．
  5. 外れたら 1 から最大 3 回やり直す（一回の control の再送で一台だけ次の秒に入ることがある，§2.2）．3 回とも外れたら，`UR-7: the devices did not take one PPS edge after 3 attempts: <理由>` で Session を拒む．
- **epoch**：N = 1 は UR-7 のまま．`time_source` が `internal` なら `set_time_now` で `Arbitrary`，そうでなければ上の手順で，`clock_source` も外部なら `Pps` になる．N ≥ 2 は常に上の手順で，epoch は `pps` になる（D-4）．
- **確かめられること（F1，M2）**：上の確認と 2026-11-27 の bench が示すのは**秒が合う**ことだけである．`last_pps` は各装置が自分で取り込んだ縁で latch した数なので，物理の差が何 tick あっても一致する．
  - PPS は参照の clock（10 MHz，100 ns の格子）で取り込まれる（§2.2）．daisy chain では PPS と 10 MHz が別の cable で届くので，取り込みの縁が一つずれると 20 tick（200 MHz）変わりうる．zero-delay の PLL も cable の遅れ（約 1 tick/m）を残す（Fable M2．INFERRED：pad と LMK の出力の遅れは読んでいない）．
  - だから装置の間の tick の差は配線ごとの性質で，S6 で測る．測るまで INFERRED と書き，仮定しない（KL，§10）．差を置く所は既にある（channel ごとの `radio.d.c<n>.path_delay_samples`（RM-23）か TM-18 の関係の uncertainty）．
  - 184.32 MHz の master clock は zero-delay ではない（`x300_clock_ctrl.cpp:604-617`）．今は binds できない（RM-28 は 200 MHz だけ）が，足すときは測り直す．
- **connect のときの coherence の確認**（Fable の item 6．orchestrator は 2026-11-27 の S3 の結果を待つ）：各 handle の `last_pps` の縁ごとの増分が `mcr` から幅（取り込みの揺れ，±20 tick と INFERRED）の中にあるかを 2〜3 縁で見れば，PPS が届かない装置，自走する別の参照に lock した装置，10 MHz と coherent でない PPS を connect のときに拒める．別々に規律された二つの参照は捕まらない．S3 が増分を印字する（§8.2）．
- **fake の要件**（UR-33，Fable M4）：`set_time_next_pps` は共有の手動の時計の次の整数秒まで時刻を変えない．「この handle の arm は一秒遅れる」fault と「この handle に PPS が来ない」fault を持たせ，やり直しと拒否の道をテストする．縁の待ちは host の時計で眠り，busy poll にしない（手動の時計は全員が idle のときに進む，`host.rs` の M2・M5）．

### 3.8 揃えた開始と巻き戻し（問い 4）

- PPS の縁を使うのは Authority を作るときの一度だけで（U5，UR-7），`arm` と `start` は今のまま．
  - `arm` は stream ごとに streamer を開く．rate の照合は装置と方向ごとの最初の open でする．送信の clock は lattice に登録する．
  - `start(T0)` は全受信 stream に `rx_start(k, T0)` を出す．T0 ≥ S = arm ＋ 2 s なので（UR-13，UR-15），timed な開始は T0 より十分前に出る．
  - 後の揃えた開始は，同じ `at`（`Session.aligned(at)`，`next_pps(k)`）を持つ stream ごとの Action である．
- **巻き戻しは stream が単位**：
  - `start` の中で一つの stream が失敗すると，`stop(Abort)` してから拒む（UR-15）．開始を出した持ち主は，untimed な stop で待っている開始を取り消す（UR-17，`ur_26_a_cut_at_a_queued_origin_stops_the_stream_at_once`）．持ち主が生まれなかった stream は `Uncertain` になり，次の open の stop で消える（UR-16，`ur_15_a_failed_rx_owner_spawn_abandons_its_started_chain_before_cleanup`）．
  - 失敗が喪失なら，以後は呼ばない（§3.4）．
  - 開始で電波は出ない．`arm` は T3 の flush だけで（UR-13），burst は `start` の後に dispatch される（KC-18）．
  - `aligned` の中の送信の burst は Action ごとに late policy に従い，装置をまたいだ「全部か無しか」は作らない（EA-16）．

### 3.9 N = 1 で変わること

健全な Run での装置の振る舞いは変わらない．変わるのは次の四つ．
1. **止まった stop の上限**（D-6）：一方向に stream が二つ以上ある layout で，1 + n_tx + n_rx 秒から約 3 s になる．
2. **Manifest の形**（D-5）：`device` section が一つだけの list になる．喪失と `CLOCK_LOST` の行は装置を持つ．`rx_segment` の `channels` は list になる（負債 #5）．
3. **binding**（D-1）：`args` が一要素の list になる．今の profile，test の binding，bench の設定を書き換える．
4. **外部の PPS での時刻合わせ**（F3）：`set_time_unknown_pps` の代わりに §3.7 の手順を通る．健全なら結果は同じで，外れたときのやり直しと拒否が新しい．

### 3.10 テストへの影響

- 葉の trait の今の method は変わらないので，N = 1 の今のテストと `impl Device` の wrapper はそのまま通る．
- 変わるのは，binding の `args` の形，`device` section を読むテスト（`ur_30_the_device_section_*` など），時刻合わせのテストである．
- `Bundle` の写しは table のテストで固め，配線の道ごとに rig のテストを一本置く．rig は note 52 D3 のとおり，二つの `FakeDevice` に一つの手動の時計を共有させる．

## 4. 利用者に見える契約

**できること**
- 一つの radio（Spec の resource 一つ）の channel を N 台の X3xx に置く．`args` は装置ごとの list で源を先頭に置き，layout の要素は `"<i>:<slot>:<board>:<frontend>"` で装置 `i` を名指す．
- connect の T0 で全装置の全 stream が始まる．後の開始は `Session.aligned(next_pps(k))` などで一つの瞬間に揃えられる．connect の後，全装置の**秒は合っている**（確かめた）．装置の間の tick の差は配線の長さで決まる性質で，S6 で測るまで INFERRED である（KL）．
- 止める：stream の node への `Stop`，radio への `Stop`，Run の終わり．どの装置の送信も，どの装置の受信より先に止まる．
- Manifest で，各装置の serial・subdev spec・source・rate の照合・時刻の確認と，失われた装置や参照の外れた装置（分からなければ `unknown`）が分かる．

**できないこと**
- 一台を失った後に残りを使い続けること（D-2）．
- 束の一台だけを別の resource として binds すること．
- 一本の stream に複数の装置の channel を入れること（D-3）．
- 種類の違う装置を一つの radio に束ねること（profile が `x3xx` だけ．root の問いは design 33 D10）．
- 10 MHz＋PPS を共有しない装置を一つの radio や一つの Run に入れること（別の Session，§12）．2 台目以降が `external` でない束は connect が拒み，別の Session を案内する．各装置の GPSDO を束に入れることもできない（D-4）．束が受けるのは分配器（全装置が `external`）と daisy chain（装置 0 が源）だけで，配線そのものは確かめられない（§3.7 の限り）．
- 装置の間の LO の位相が tune や open をまたいで再現すると期待すること（timed tune が無い，RM-9．Fable §4）．
- 喪失の後に残りの装置がすぐ無音になると期待すること（渡した 10 ms 以内は出る，§3.4）．
- connect が N = 1 と同じ時間で終わると期待すること．open は一台ずつ直列で，受け手の切り替えは lock まで最大 30 s かかる（m6，KL）．

## 5. 退けた案（理由つき）

| 案 | 理由 |
|---|---|
| A：一つの compound（`addr0=…,addr1=…`） | 装置をまたぐ揃えを実機で測るまでは，またぐ stream を拒むことになる（fake は測っていない振る舞いを模せない，UR-33）．その間に残るのは UHD の時刻合わせだけで，compound を模す新しい fake，全装置で一つの mutex，同じ種類の装置だけ，を払う値打ちが無い．並列の初期化が daisy chain で安全かも未確認 |
| G を今（要素に compound も許す） | 二段の番号，同じ束の二つの書き方とその両方のテスト，compound の fake を今払う理由が無い．揃えを測った後に，Module だけの変更で足せる |
| 合成を置かず，Provider が装置を選ぶ | Provider の装置の呼び出し約 66 か所がすべて装置を選ぶことになり，取り違えの場所が増える |
| 装置ごとの小さな Provider を一つの instance の裏に | `ActionReceiver` は一つなので振り分け役が要り，note 52 で退けた「controller の一本」（Stop が遅れる，49 §5.14）になる |
| 装置ごとのスレッド，装置ごとの uhd-control | 塞がる `send` が同じ装置の他の stream を止める（U2）．`ActionReceiver` と KC-21a は instance に一つ |
| 装置ごとの stop（直列か並列） | RM-16 の「送信が先」を装置をまたいで破る．直列なら最悪が N × 3 s |
| スレッドごとの 1 s を残し，Provider の締め切りを申告する | hook が Provider に配線されておらず，Kernel の変更になる．最悪も N と共に伸びる |
| 装置ごとの喪失（健全な装置は普通の stop） | 見直しの最も多い喪失の機構を装置ごとに作り直す．取り違えると呼び出しが塞がる．D-2 で後へ（閉じない約束） |
| 失った handle だけに印を付け，健全な handle を解放する | 喪失の道 10 か所は装置を知らず，ふつうの喪失（沈黙，読みの失敗）では帰属が分からない．取り違えると teardown が死んだ link に書き，process が落ちうる（Fable M3）．server では利用者に見える得が無い（F2） |
| N = 1 だけ今の `set_time_unknown_pps` と合成なしの道 | 一つの規則の二か所の実装になり，N ≥ 2 の道は 2 台の bench でしか通らない（Fable M4，F3） |
| 時刻合わせの確認が外れたら一回で拒む | 一回の control の再送で一台が次の秒に入る．縁をまたいだ読みで健全な束も拒む（Fable M4，F3） |
| 葉の trait の method に装置の番号を足す，既定の `timekeeper(i)` | 十数個の wrapper が変わる．既定で自分を返す形は Rust で書けず，書けても wrapper が束を隠す（m0） |
| 源の REF OUT と PPS OUT を出す呼び出しを Module に置く | UHD が開くたびに ON にする（§2.2）．今は要らない．S3（= S4a）が外れ，S3d（明示で ON）で通るなら足す |
| 同期していない radio を一つの Run の別の root に | owner の方針で別の Run にした（§12）．後で要るなら RS-19，SB-16，SC-23b を変える凍結後の Kernel の変更になる（INFERRED） |

## 6. 判断の原則

1. **装置の束は `Bundle` の下に隠す**．Provider が見る `Device` は一つで，装置ごとの事実は `Bundle` の具体の method で持つ．
2. **持ち主の単位は stream のまま**．装置は事実の持ち場であって，スレッドも lock も持たない．
3. **喪失は instance に一つ**．公表の後はどの装置も呼ばず，全 handle を保持する．
4. **uhd-control の一回の仕事は N によらない**．
5. **止める順は instance 全体で，上限は N によらない**．
6. **一つの規則は一か所**：外部の PPS での時刻合わせは N によらず一つの手順で，N = 1 も `Bundle` を通る．
7. **確かめたことだけを約束する**：秒の一致は connect で確かめる．tick の差は配線の性質として測り，測るまで INFERRED と書く．

## 7. 既存の約束との照合

| 約束 | 結果 |
|---|---|
| Kernel（MA-9a，MA-14b，KC-21a，RS-8a，TM-16a） | 変更なし．Kernel の finding は無い |
| KC-46b の「約 3 s」 | D-6 の後はどの layout でも正しい |
| TM-18 | timekeeper 1…N−1 の root との関係（秒の一致の確認の結果と，S6 で測る差）は，Provider の `device` section の装置ごとの行に書く |
| EA-7 | binding は一つのまま．server は要素ごとに `Config.open_device` を呼ぶ．拒否の文 `one USRP per Run` は「一つの binding」に直す（文だけ） |
| EA-10，EA-16 | `next_pps` は epoch `pps` で使える（D-4）．`aligned` は変わらない．多くの呼び出しには `at` の余裕が要る |
| RM-11，RM-16 | `CLOCK_LOST` の source は `<device>` のまま，どの装置かは D-5 の行に書く．送信が先は instance 全体で守る |
| RM-28 | 「timekeeper は 0」の拒否を外す段で，MockRadio にも D-3 の拒否を入れる（Mock が実機より緩くならないように．Fable §4） |
| RM-32 | 上限が link 1 本ぶんの instance に一つなので，装置ごとに link を持つ N 台では厳しすぎる．語彙の段への finding |
| UR-2，UR-3，UR-4 | UR-2 は §3.1 の二つを足す．UR-3 の mutex は handle ごと．UR-4 は要素ごとに `open` |
| UR-5 | serial と master clock は装置ごと．同じ装置の二度書きは開く前に拒む（m4） |
| UR-6，UR-7，UR-8 | Authority は `Bundle` を共有し，§3.7 の手順で時刻を入れる．`reopen_on_unlock` は束ごと（m3）．anchor と関係は handle 0 |
| UR-13，UR-14，UR-27，UR-29 | rate の照合は装置と方向ごと．点検は一回に一台．`CLOCK_LOST` は装置ごとに一回．1 s の失敗は装置ごと |
| UR-15，UR-16，UR-17，UR-26 | 開始と巻き戻しは今のまま．join の上限は段ごと（D-6） |
| UR-30 | `device` section が装置ごとの行を持つ（D-5） |
| UR-33 | fake は今の一台を N 個と共有の手動の時計で，時刻合わせの二つの method と fault を足す（§3.7） |
| KL-1 | 開く装置が N 台なので，lock の失敗は 1 − 0.96^N に増える（N = 2 で約 8 %．INFERRED：算術） |
| KL-2 | どの装置が落ちても radio が失われ，全 handle が process の終わりまで保持される |
| KL-6，KL-15 | 外部参照と揃えた開始は 8b の bench で初めて試す．lead の値は一台・1 channel で測ったもので，N > 1 では INFERRED |
| design 33 §3 | 「UHD で 1 回だけ開く」は要素ごとに 1 回と読む．「stream は timekeeper をまたいでよい」は宣言しない側をとる |
| note 52 の負債 #5，#20 | #5 は D-5 と一緒にそろえる．#20 は D-6 で閉じる |

## 8. 実機の証拠と bench

| 主張 | 状態 | 確かめる所 |
|---|---|---|
| 開き終えた X300 は REF OUT と PPS OUT を出す | VERIFIED（source，§2.2） | S3（= S4a），S3d，S4b |
| 受け手が lock した後で源を開くと，受け手の参照が途切れる | INFERRED（Fable M1） | S5a |
| 源が出力していない間に受け手を `external` で開くと失敗する（v3 の失敗） | INFERRED | S5b |
| `internal` で開いてから源を先に切り替えると，受け手が lock して秒が合う | INFERRED | S3（10 回） |
| 時刻の確認が PPS の欠けを捕まえる | INFERRED（手順） | S4b |
| `last_pps` の増分は `mcr` から ±20 tick に収まる | INFERRED（Fable M4） | S3 の印字（item 6 の判断に使う） |
| 装置の間の tick の差は配線で決まり，open をまたいで 20 tick 跳びうる | INFERRED（Fable M2） | S6（10 回） |
| 喪失の後，健全な装置の受信は overflow で止まり，送信は黙る | INFERRED | 後の段（2 台で受信中に cable を抜く） |
| 5 ms と 10 ms の lead は N = 2・複数 stream でも保たれる | INFERRED（KL-15） | 後の段 |

### 8.1 2026-11-27 の手順（UHD と `sync_check` だけ．USRP の送信なし）

- **何を決めるか**：B の手順（`internal` で開き，源を先に切り替え，一つの縁で時刻を入れる）が lock し，秒が合うか．開き方の危険（S5）は記録だけにする．S6 は装置の間の tick の差を測る．11-27 は Module の code を一行も通らない．`sync_check` が手順を書き写すので，記録には「UHD の振る舞いを確認．Module の道は未試験」と分けて書く．
- **owner の確認が要るもの**：USRP の送信はどの段にも無い．S6 だけは外の信号源を RX に入れるので，各 RX port に入る level を owner が確かめる．#1 の `ref_locked` が false のままなら（S0），REF OUT・PPS OUT と入力の cable を付け替えて #2 を源にする．これは owner の手の作業で，以後の `A1` と `A2` を入れ替える．
- **回数**：各段 10 回．一回が新しい process（新しい container）である．KL-1 の lock の失敗（open あたり約 4 %）が出たら，その回は数えずに記録だけ残す．
- **準備**：`sync_check` は 11-27 より**前に** build し，`sc probe` を一台に一度流して，image に `cc` と libuhd への link の道があることを確かめておく（2026-11-21 に image で build と `selftest ok` を確かめた．`sc probe` は装置が要るので 11-27 の前日準備で）．source は木の中に置き，read-only で mount する．binary と log は木の外の `$L` に置く（Safety core：build の出力は木の外）．container は自分の uid で走らせる（bench.md と同じ `--user`）．

```sh
plan/phase8/tools/bench-host.sh --nic enp1s0f1np1 --subnet 40     # #1．bench.md BP1 のとおり
plan/phase8/tools/bench-host.sh --nic enp1s0f0np0 --subnet 47     # #2．subnet の値は bench.md に合わせる（INFERRED）
S=$PWD/plan/phase8/tools/sync_check                               # sync_check.c の置き場（§8.2）
L=~/.cache/ezsdr-bench/2026-11-27-8b0; mkdir -p "$L"              # binary と log（木の外）
docker run --rm --user "$(id -u):$(id -g)" -v "$S:/src:ro" -v "$L:/out" ezsdr-v4-dev:uhd4.10 \
  cc -O2 -std=c11 -Wall -o /out/sync_check /src/sync_check.c -I/usr/local/include -L/usr/local/lib -Wl,-rpath,/usr/local/lib -luhd -lpthread -lm
sc() { docker run --rm --network=host --user "$(id -u):$(id -g)" -v "$L:/out" ezsdr-v4-dev:uhd4.10 /out/sync_check "$@"; }
A1=addr=192.168.40.36; A2=addr=192.168.47.37
```

| 段 | 命令（各 10 回） | 読むもの | 期待 | 外れたら |
|---|---|---|---|---|
| S0 | `docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_find_devices` を一回．`for i in $(seq 10); do sc probe --run $i "$A1"; sc probe --run $i "$A2"; done \| tee "$L/s0.log"` | `serial`，`mcr`，`ref_locked`，`open_ms` | 両方が開き，`mcr=200000000`，`ref_locked=1` | #1 が `ref_locked=0` なら止め，owner が cable を付け替えて役を入れ替える |
| S3（= S4a） | `for i in $(seq 10); do sc separate --run $i "$A1" "$A2"; done \| tee "$L/s3.log"`．Module の手順そのもので，source out を呼ばない．[01-device-bundle-decisions.md](01-device-bundle-decisions.md) と Fable の S4a（「source out を呼ばずに PPS が届く」）はこの段が兼ねる | `lock1_ms`，`attempts`，`last_pps`，`tnow_delta_ns`．増分 `incr0`，`incr1` は記録だけ（幅は INFERRED で，それを測る） | `verdict=PASS` 10/10．`attempts=1` が多い | S3d へ |
| S3d（S3 が `REFUSED` のときだけ） | `for i in $(seq 10); do sc separate --run $i --pps-out on "$A1" "$A2"; done \| tee "$L/s3d.log"`（#1 の PPS OUT を明示で ON にする） | `verdict`，`reason` | S3d が通るなら M1 の読みが実機で外れた．Module に source out の呼び出しを足す．S3d も外れるなら配線か lock を疑う（`ref_locked`，S0） | — |
| S4b | `for i in $(seq 10); do sc separate --run $i --pps-out off "$A1" "$A2"; done \| tee "$L/s4b.log"`（#1 の PPS OUT を切ってから時刻を入れる．次に #1 を開けば UHD が ON に戻す） | `attempts`，`reason` | `verdict=REFUSED` 10/10（3 回やり直して拒む）．確認が欠けを捕まえる negative control | 確認が欠けを見逃した．手順を直す |
| S4c | `for i in $(seq 10); do sc separate --run $i --order 10 "$A1" "$A2"; done \| tee "$L/s4c.log"`（#2 を先に開く） | S3 と同じ | `PASS` 10/10．効くのは開く順ではなく切り替える順（§3.7） | 開く順が効く．B の「どれも `internal` で開く」の読みを見直す（M1 の当否とは別） |
| S5a | `for i in $(seq 10); do sc reopen-source --run $i "$A1" "$A2"; done \| tee "$L/s5a.log"`（#2 を `external` で lock させてから #1 を開き，その間の #2 の `ref_locked` を 10 ms ごとに読む） | `false_samples`，`first_false_ms`，`relock_ms` | 記録だけ（INFO）．途切れが出れば m3・m5 の KL の根拠になる | — |
| S5b | `for i in $(seq 10); do sc outputs-off --run $i "$A1"; sc follower-first --run $i "$A2"; sc probe --run $i "$A1"; done \| tee "$L/s5b.log"`（#1 の出力を切って閉じ，#2 を `external` の args で開き，最後に #1 を開いて出力を戻す） | `open`，`error`，`ref_locked` | 記録だけ（INFO）．v3 の失敗が出るかを読む | — |
| S1，S2（任意．G のため） | `C=addr0=192.168.40.36,addr1=192.168.47.37; for v in "$C" "$C,clock_source1=external,time_source1=external" "$C,serialize_init"; do for i in $(seq 10); do sc compound --run $i "$v"; done; done \| tee "$L/s12.log"`．先に一回 `docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args "$C"` | `synchronized`，`last_pps`，`ref_locked` | 記録だけ．後で G の compound の要素を足すときの根拠 | — |
| S6（信号源が要る） | owner が雑音源か変調した信号を splitter と att で #1 と #2 の RX2 に入れ，level を確かめる．`for i in $(seq 10); do sc align --run $i --freq 1e9 --rate 200e6 --samps 65536 --gain 0 "$A1" "$A2"; done \| tee "$L/s6.log"` | `lag_ticks`，`peak_db` | 記録だけ．10 回の分布で，固定の差と 20 tick の跳びの有無を読む | — |

- **所要**：S0〜S5b で約 1 時間，S1・S2 で約 30 分，S6 で配線を除き約 15 分（INFERRED）．
- **記録**：log は `$L` に残す．後で `bench/2026-11-27-8b0/` に，`conditions.json`，log から作った `results/`，`README.md`，`raw.txt` を置く（AGENTS §9 の bench の記録）．S3 の増分は item 6 の判断に使う（§10）．

### 8.2 `sync_check.c` の仕様（実装者向け．実装は `plan/phase8/tools/sync_check/`，b1f86d86）

- **形**：C11 の一 file で，UHD の C API（`uhd.h`）だけを使う（250〜350 行，INFERRED）．**送信の API（`uhd_tx_*`，`uhd_usrp_get_tx_stream`）を一度も参照しない**．review では `grep -n tx_ sync_check.c` が 0 件であることを見る．置き場は `plan/phase8/tools/sync_check/sync_check.c`（orchestrator が決める）．
- **呼び方**：`sync_check MODE [--run N] [options] ARGS...`．終了の status は，`PASS` と `INFO` が 0，`FAIL` と `REFUSED` が 1，`ERROR` が 2．
- **出力**：一回に一行を stdout に出す．先頭が段の名前で，後は空白区切りの `key=value`（list は `,` 区切り）．時間は process の開始からの ms（`CLOCK_MONOTONIC`）．UHD の呼び出しが失敗したら，`error="<call>: <uhd_usrp_last_error か uhd_get_last_error の文>"` と `verdict=ERROR` を出して終わる．
- **共通の部品**：
  - `open(args, &h)`：`uhd_usrp_make` の時間を測る．
  - `ref_locked(h)`：`uhd_usrp_get_mboard_sensor(h, "ref_locked", 0, …)` を `uhd_sensor_value_to_bool` で読む．
  - `lpps(h)` と `tnow(h)`：`uhd_usrp_get_time_last_pps` と `uhd_usrp_get_time_now` の値を mboard 0 で，`full·200000000 + round(frac·200000000)` の tick にする．
  - `wait_edge(h, timeout_ms)`：`lpps(h)` が変わるまで 1 ms ずつ `nanosleep` する．
  - `switch_follower(h)`：`uhd_usrp_set_clock_source(h, "external", 0)`，`uhd_usrp_set_time_source(h, "external", 0)` の後に，`ref_locked` を 50 ms ごとに最大 30 s 読み，lock までの ms を返す．
  - `set_time(hs[], n)`：§3.7 の手順をそのまま写す（縁を待つ，その秒に全 handle へ `uhd_usrp_set_time_next_pps(h, 0, 0.0, 0)`，次の縁を待つ，handle 0 の二回で挟んで確かめる，最大 3 回）．試行の数，挟み直しの数，最後の理由を返す．
  - `incr(hs[], n, k)`：handle 0 の縁を k 回待ち，縁ごとに全 handle の `lpps` を読んで差を返す．
- **MODE ごとの手順と出力行**：
  - `probe A`：開いて読むだけ．`S0 run= dev= open_ms= serial= mcr= ref_locked= time_now= last_pps= verdict=PASS`．serial は `uhd_usrp_get_rx_info(h, 0, …)` の `mboard_serial`．
  - `separate [--order 01|10] [--pps-out on|off] A0 A1`：`--order` の順に両方を `internal` で開く．`--pps-out off` なら h0 に `uhd_usrp_set_time_source_out(h0, false, 0)` を，`--pps-out on` なら `true` を出す．どちらも無ければ何も呼ばない（Module と同じ）．h1 を `switch_follower` し，`set_time` と `incr(…, 3)` をする．出力行は `S3 run= order= pps_out=none|on|off open_ms=o0,o1 lock1_ms= ref_locked=r0,r1 attempts= bracket_rereads= last_pps=l0,l1 tnow_delta_ns= incr0=a,b,c incr1=a,b,c verdict=PASS|REFUSED reason=`．先頭は，`--pps-out on` なら `S3d`，`--pps-out off` なら `S4b`，`--order 10` なら `S4c` にする．
  - `reopen-source A0 A1`：h1 を `internal` で開いて `switch_follower` する．別の thread で h1 の `ref_locked` を 10 ms ごとに読みながら，h0 を開く．出力行は `S5a run= lock1_before= open0_ms= samples= false_samples= first_false_ms= relock_ms= verdict=INFO`．
  - `outputs-off A0`：開き，`uhd_usrp_set_clock_source_out(h, false, 0)` と `uhd_usrp_set_time_source_out(h, false, 0)` を出して閉じる．`S5b-off run= verdict=INFO`．
  - `follower-first A1`：`A1` に `,clock_source=external,time_source=external` を足して開く．失敗してもよい．`S5b run= open=ok|error error= ref_locked= verdict=INFO`．
  - `compound ARGS`：一つの compound として開く．args が source を書かなければ mb1 を `external` に切り替える．`uhd_usrp_set_time_unknown_pps`，`uhd_usrp_get_time_synchronized` を呼び，mboard ごとの `lpps` を 3 縁読む．`S2 run= args= open_ms= lock1_ms= ref_locked=r0,r1 synchronized= last_pps=l0,l1 incr0= incr1= verdict=INFO`．
  - `align [--freq F] [--rate R] [--samps K] [--gain G] A0 A1`：`separate` の手順の後で，両方の RX channel 0 を F，R，G と antenna `RX2` に合わせる（`uhd_usrp_set_rx_antenna(h, "RX2", 0)`）．t = `tnow(h0)` ＋ 0.5 s に，両方へ `STREAM_MODE_NUM_SAMPS_AND_DONE` の K samples の timed な開始を出して受ける．|相互相関| の最大を ±64 sample で探す．`S6 run= rate= samps= t_start= lag_samples= lag_ticks= peak_db= second_peak_db= verdict=INFO`．

## 9. この決定で後からできなくなること，後の段に回すこと

- **できなくなること**：
  - 装置をまたぐ一つの block（G の compound の要素として，揃えを測った後に足せる）．
  - UHD 自身の一つの縁での時刻の設定（Module がなぞる）．
  - 一部の装置で Run を続けること（D-2．致命的でない装置ごとの event の道は閉じない）．
  - 一つの Run に root を複数持たせること（§12．後で要るなら凍結後の Kernel の変更）．
- **閉じないもの**：
  - 種類の違う装置を束ねる道（root の問いは別）．
  - 8c の Replay（装置ごとに graph が一つ．喪失の後の stop の道を一緒に足す，§3.4）．
  - HackRF・bladeRF の Module（compound を持たない driver に同じ形が移る．INFERRED）．
- **後の段**：
  - selector の key の細部（装置ごとの list の名前と既定）．
  - RM-28 の timekeeper の拒否を外す段と MockRadio の複数台（D-3 の拒否を Mock にも）．
  - 装置の間の揃えの宣言（Vision §25 の coherence と，S6 の後の差の上限）．
  - RM-32 の上限を装置ごとにすること．
  - `t<i>` の node（D-5 の代わりに後で）．
  - connect のときの coherence の確認（item 6，§10）．

## 10. 決定の記録と，まだ owner の手が要るもの

- **owner の決定**（2026-11-20，[01-device-bundle-decisions.md](01-device-bundle-decisions.md)）：
  - D-1：「はい、よいです」．B，N = 1 は一要素の list，G は後．
  - D-2：「じゃあこれで良いです。問題があれば後で」．KL に一つ足し，装置ごとの喪失は閉じない．
  - D-3：「推奨でokです」．D-4：「推奨で良いです」．混ざった構成は別の Run で，各装置の GPSDO は今は許さない．
  - D-5：「ok」．D-6：「ok」．
  - F1：「そもそもPPS/10MHzを配る配線長で変わるので」．F2：「okです」．F3：「okです」．
- **orchestrator の決定**：
  - Fable の item 4（分かれば名指し，分からなければ `unknown`）を採る．
  - item 6（connect のときの coherence の確認）は 11-27 の S3 の増分を見てから決める．
  - item 7（各段 10 回，S4a/S4b，役の入れ替え）は bench の手順に入れた（§8.1．S4a は S3 が兼ね，切り分けの S3d と開く順の S4c を足した）．
- **この改訂で orchestrator に確かめてほしいもの**：
  - Module に REF OUT と PPS OUT の呼び出しを置かないこと（§5．S3 が外れ，S3d で通るなら足す）．
  - m0 の (b)（`Bundle` を具体の型で渡す）．
  - N = 1 も `Bundle` を通し，differential の rig で今と比べること．
- **まだ owner の手が要るもの**：
  1. **D-5 の文の狭め**：owner の D-5 は「message が装置を名指す」だったが，panic などでは装置が分からず `unknown` になる（orchestrator が採った）．Manifest の中身なので，一言確かめる．
  2. **KL の文**（owner が決める）：
     - D-2 の「どの装置の喪失でも radio が失われ，全 handle を process の終わりまで保持」．
     - F1 の「装置の間の tick の差は PPS/10 MHz の配線の長さで決まり，測るまで INFERRED」．
     - m1 の「一台が塞がると全装置の stream のスレッドが detached になりうる」．
     - m5 の「別々の Run が daisy chain で参照を分け合うなら，分配器を使うか，源の Run を先に開いて最後まで保つ」．
     - m6 の「connect は N 台で長くなる（受け手の lock に最大 30 s）」．
  3. **11-27 の当日の作業**：#1 の `ref_locked` が false のままなら cable を付け替えて役を入れ替える．S6 には信号源，splitter，att が要り，RX に入る level を owner が確かめる．信号源が無ければ S6 は別の日にする．
  4. **item 6** の判断（11-27 の後）．
  5. **各装置の GPSDO** を束に許すか（望むなら別に）．

## 11. Fable の指摘の扱い

| 指摘 | 扱い |
|---|---|
| M1 PPS OUT は開くと ON | 採った．§2.2，§2.3，§3.7，§5，§8 を直した．S4 は S4a（S3 が兼ねる）と S4b に分け，切り分けの S3d と開く順の S4c を足した |
| M2 11-27 が決めるのは秒の一致まで | 採った（F1）．§3.7，§4，§8 に書いた．S6 は 10 回で，USRP の送信なし．S0 の役の入れ替えには cable の付け替えが要る |
| M3 喪失の帰属と解放 | 採った（F2 と item 4）．全 handle に印を付けて保持し，分からなければ `unknown` |
| M4 時刻合わせ | 採った（F3）．3 回のやり直し，handle 0 で挟む確認，N = 1 も同じ道．fake の要件も足した |
| m0 `timekeeper(i)` の既定は書けない | 採った．(b) で，`Bundle` を具体の型で渡す（§3.1） |
| m1 gate は数 ms ではない | 採った．§3.3 の文を直し，KL の文を一つ足す |
| m2 単数の method の意味 | 採った（§3.1） |
| m3 `reopen_on_unlock` は束ごと | 採った（§3.7） |
| m4 同じ装置の二度書き | 採った．開く前に find で拒む（find の `serial` は INFERRED） |
| m5 別の Run が参照を分け合う | 採った．KL の文と S5a |
| m6 connect の時間 | 採った（§4，KL の文） |
| m7 行番号 | 記録だけ．`e008038b` の行で，Fable が `ea7bc5c1` で確かめた範囲は合っていた |
| §4 の注意（8c の Replay，Mock の D-3，LO の位相） | 採った（§3.4，§7 の RM-28，§4） |

## 12. 同期していない radio は別の Run（owner 2026-11-20）

- **Kernel も凍結前の仕事も要らない**（VERIFIED）：
  - `connect()` は Session ごとに server process を一つ起こす（`python/ezsdr/_client.py:70`）．server は一つの Session を stdin と stdout で扱うので，port は無い（`crates/ezsdr-server/src/main.rs:1, 66`）．
  - `SerialGuard`（`provider/mod.rs:120-146`）と UHD の device の map，kept の list（`uhd.rs:621-629`）は process ごとにある．同じ装置を二つの Session で開くと，UHD の claim が二つ目を拒む（§2.2）．
  - Session の directory は `session-<pid>-<n>`（`crates/ezsdr-server/src/lib.rs:405-425`）．server が読む環境変数は `EZSDR_PROFILE` と `EZSDR_SERVER_FAILSTOP_SECS` だけ（`main.rs:74-75`）で，fail-stop は入力が終わったときにだけ動く（`main.rs:1-7, 37-54`）．
  - 本体の source で host 全体の資源を使うのは acceptance の rig の一つだけ（`ezsdr-acceptance/src/rig.rs:23`）．Python の package の module 直下は定数だけ．
  - 両方の Session が既定の profile に頼ると同じ装置を開こうとするので，`connect(profile=…)` で渡す（INFERRED）．二つの Session を実際に同時に流したことは無い．
- **一つの Run に比べて失うもの**：
  - Manifest が二つになり，時刻は各 Manifest の root から `utc` への関係でだけ結べる（`host.monotonic` の原点は Authority ごと，UR-7）．
  - 一つの Spec で両方をつなげない．
  - `Connection` は一度に一つの要求だけを扱うので，thread を分ける．
  - 同期していない組では，一つの Run にしても時刻は TM-5 の不確かな変換でしか結べないので，時刻の上で失うものは無い．
- **一つの Run が一つの時刻の土台を持つ規則**はそのまま残る．RS-19 の `earliest`（`coordinator/pipeline.rs:1199-1204`），SB-16（`design/03-spec-and-binding.md:186`），SC-23b（`design/02-stream-contract.md:244`）．
- **シミュレーション**：複数の模擬 radio は今も一つの Run の一つの root で動く（CH-6，MR-31 の `with_medium`，`crates/ezsdr-acceptance/tests/reactive.rs:156-190`，VERIFIED）．同期していない radio の模擬（CFO，sample clock のずれと drift，装置ごとの時刻）は，一つの root に対する impairment として凍結の後に足せる（INFERRED．今は C4 と §8 で外している，`design/11-simulation-channel.md:114, 154`）．

## 付録：調べ方

- main の読み：`git show main:<path>` で spec 01〜07・16・18，`KNOWN_LIMITATIONS.md`，`handoff.md`，`crates/ezsdr-radio-uhd/src/{uhd.rs,authority.rs,provider/*}`，`crates/ezsdr-kernel/src/{run.rs,coordinator/ending.rs,coordinator/pipeline.rs}`，`crates/ezsdr-server/src/{main.rs,lib.rs}`，`python/ezsdr/{_client.py,session.py}`．
- uhd-control の装置の呼び出し：`sed -n '234,644p' control.rs | grep 'device\.'` は 0 行．`deadline_millis` の上書き：`git grep -n 'fn deadline_millis' main -- crates` は `run.rs:518` と test の double だけ．
- UHD v4.10.0.0（`gh api … -H 'Accept: application/vnd.github.raw'`）：`host/lib/usrp/{multi_usrp_rfnoc.cpp,x300/x300_impl.cpp,x300/x300_prop_tree.cpp,x300/x300_mb_controller.cpp,x300/x300_regs.hpp,x300/x300_clock_ctrl.cpp,x300/x300_defaults.hpp,x300/x300_device_args.hpp,x300/x300_claim.cpp,x300/x300_fw_ctrl.cpp}`，`host/include/uhd/{usrp/usrp.h,utils/soft_register.hpp}`，`fpga/usrp3/top/x300/{x300.v,x300_core.v}`，`fpga/usrp3/lib/timing/pps_synchronizer.v`，`host/docs/driver_usage/{multiple,sync}.dox`，`host/docs/devices/usrp_x3x0.dox`，`host/examples/rx_multi_samples.cpp`．`x300_regs.hpp` の `PPS_OUT_EN` の既定値は v4.0，v4.4，v4.6 でも見た．Fable が読んで私が読んでいないもの：`property_tree.ipp:80-107`，`bus_int.v:386-391`，`usrp_c.cpp:215-218, 368, 384`，`radio_rx_core.v:586-595`，`x300_clock_ctrl.cpp:604-617`．
- 「五つの Kernel の規則」（判断 1 が退けた案）：`git grep -n -i 'five Kernel rules\|relation hook\|root per device' main -- plan design` の当たりは判断の行だけで，規則の名前は記録に無い．§12 で問いを閉じた．
- 確かめていないこと：
  - X300 の REF OUT が開いている間に途切れること（S5a）．
  - UHD の find の結果に `serial` が入ること（m4）．
  - stream command の道の timeout（m1）．
  - image に UHD の Python API があるか（`sync_check` は C にしたので要らない）．
  - 受信の flow control が radio まで背圧を返すこと．
  - 二つの Session の同時の実走．

## 13. 段の分け方（orchestrator 2026-11-20）

各段は木を緑のまま残す．11-27 の bench の結果に頼る段は bench の後に置く．

| 段 | 中身 | risky | bench の前に始められるか |
|---|---|---|---|
| 8b-1 | D-6：`Provider::stop` の join を段ごとに共有する 1 s にする（uhd-control，全送信，全受信）．spec 18 UR-16・UR-26 と spec 06 KC-46b の文．負債 #20 を閉じる | はい（期限） | はい |
| 8b-2 | N = 1 の `Bundle`：binding の `args` を list に，葉の trait に `time_last_pps`・`set_time_next_pps`，`Bundle` の写しと装置ごとの事実，Manifest の `device` を装置ごとの行に（D-5，負債 #5），喪失の名指し | はい（FFI の境界，fake） | はい |
| 8b-3 | §3.7 の時刻合わせ（F3）と fake の PPS の fault．item 6 を足すかはここで決める | はい | いいえ（S3 の結果を待つ） |
| 8b-4 | N ≥ 2：list の順に開く，source の規則（D-4）と拒否，serial の重複，一回に一台の点検，D-3 の拒否，二つの FakeDevice の rig，KL の文（owner） | はい | いいえ（S4・S5 の結果を待つ） |
| 8b-5 | MockRadio の複数台と RM-28 の timekeeper の拒否を外す | はい | はい（8b-4 の後） |
