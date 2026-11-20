# 8b step 0（④）の設計 review — Fable 5.2

- reviewer：Claude Fable 5.2（`claude-fable-5-2`）．AGENTS §9 step 0 の adversarial design review．費用は呼び出し側が報告する．
- 対象：`draft.md`（Opus，根拠は `e008038b`）と `decisions.md` の D-1…D-6．コードは `main` `ea7bc5c1`（作業木は clean）を読み，行番号はこの commit のもの．UHD は v4.10.0.0 を `gh api repos/EttusResearch/uhd/contents/<path>?ref=v4.10.0.0` で読んだ．木は変えていない．
- **順序の申告**：自分の失敗シナリオ（§1）は，draft を読む前に `uhd.rs`・`device.rs` の `Device` trait・`authority.rs`・`core.rs` の gate を読んで作ったが，**書き出したのは draft を通読した後**である（指示の順からの逸脱）．UHD の source の照合はすべて draft の後で，draft の主張とは独立に file を取り直した．§1 の各項に「draft 前／後」を付けた．

## 0. 結論（先に）

**方向 B は維持でよい．開き直す理由は見つからなかった．ただし draft のままでは進めない点が major 4 件ある．** 特に M1 は，D-1 と D-4 の文に入った「UHD は開くたびに PPS OUT を切る」が誤りで（VERIFIED とされているが違う），bench の S4 の期待値が逆になる．M2 は，2026-11-27 の bench（`sync_check`，送信なし）では D-3・D-4 の「同じ tick」を決められないこと．M3 と M4 は B の合成 `Device` の喪失と時刻合わせの設計で，どちらも安い直し方がある．

Kernel の finding は無い．読んだ範囲：`crates/ezsdr-kernel/src/run.rs:505-580`（`CleanupOps`，`DEFAULT_CLEANUP_DEADLINE_MS = 5_000`，`run_cleanup` の step ごと・fragment ごとの締め切りと放棄），`coordinator/ending.rs:335-356` と KC-39・KC-44 の行，`provider/mod.rs:729-790`（`stop` と `cleanup`）．draft §2.1・§2.4 の事実（締め切りは step ごと 5 s，放棄された step の slot は KC-39，`stop` の join は一本ごとに 1 s）はこの範囲で合っていた．Kernel の他の部分（pipeline，RS-19 など §12 が引く規則）は読んでいない．

## 1. 自分の失敗シナリオと異議（draft と比べる前の list）

| # | シナリオ／異議 | 時点 | 結果 |
|---|---|---|---|
| O1 | 一台を失った後，健全な装置は誰が止めるか．`UhdDevice::drop` は健全な handle を「止めていない stream を持ったまま」解放する | 前 | M3 |
| O2 | handle をまたぐ PPS の時刻設定：N 回の control 呼び出しが縁をまたぐ／一回の UDP の再送（1 s）で一台だけ次の秒に入る／確認の読みが縁をまたぐ | 前 | M4 |
| O3 | `Device` trait に装置の次元が無い（`serial`，`set_sources`，`mark_lost`，`master_clock_rate`，`set_subdev_spec` が一つ）．合成の裏に隠せるのは stream と channel だけ | 前 | M3，m0，m2 |
| O4 | 一つの `device_gate` を握ったまま装置を呼ぶ（`core.rs:1553-1557`）．死にかけの一台の呼び出しが健全な装置の開始と stop を待たせる | 前 | m1 |
| O5 | `uhd-clock` は一台しか読まない．他の装置の時刻が外れても root は気づかない | 前 | M4 の確認に吸収．Run 中の再確認は不要（時刻は一度しか入らない） |
| O6 | process-global：`KEPT`・`LIVE`（`uhd.rs:624, 629`），`uhd_get_last_error`，C API の handle の map．二つの Session が同じ process なら data race | 前 | 該当せず：server は Session ごとに一 process（draft §12(1)，受け入れる） |
| O7 | 同じ X310 を二つの address（SFP0 と SFP1）で list に二度書く．UHD の claim は同じ process を通すので二つ目の open が一つ目の下の装置を初期化し直す | 前 | m4 |
| O8 | PPS は各装置が自分の clock で取り込む．配線の遅れと取り込みの縁の曖昧さが「同じ tick」に残る．`last_pps` の一致では見えない | 前（機構の特定は後） | M2 |
| O9 | daisy chain の源を開き直す（`reopen_on_unlock`，別の Session）と REF OUT と PPS OUT が途切れ，follower の lock が外れる | 後（advisor の指摘で確認） | m3，M2 の bench 項目 |
| O10 | 8c：host が供給しない送信（Replay）は「以後どの装置も呼ばない」では止まらない | 前 | §4 |
| O11 | 装置をまたぐ位相（LO）の再現性は timed tune が要るが，`apply` は「now」だけ（`device.rs:185`） | 前 | 範囲外．§5 に一行 |

## 2. Major

### M1. 「UHD は装置を開くたびに PPS OUT を切る」は誤り（draft §13.1 は VERIFIED と書くが，読み落としがある）．D-1・D-4 の文と bench S4 の期待値が逆

- **証拠（VERIFIED，source）**：draft が読んだとおり，`x300_mb_controller` の構築は clock 制御 register を既定値（`PPS_OUT_EN = 0`）で書く（`x300_regs.hpp:217-225`，`x300_mb_controller.cpp:71-72`）．しかしその直後に `x300_impl::setup_mb` が `init_prop_tree` を呼び（`x300_impl.cpp:374`），そこで
  `pt->create<bool>(mb_path / "time_source" / "output").add_coerced_subscriber([mbc](const bool o){ mbc->set_time_source_out(o); }).set(true);`（`host/lib/usrp/x300/x300_prop_tree.cpp:64-68`，コメントは "setup the time output, default to ON"）が走る．つまり**開き終えた X300 の PPS OUT は ON**である．draft の付録の読んだ file の list に `x300_prop_tree.cpp` は無い．`set()` は coercer が無ければ `_set_coerced(value)` を呼び，それが coerced subscriber を全部呼ぶ（`host/include/uhd/property_tree.ipp:80-107`）．source の上では端から端まで VERIFIED で，実機では実行していない（S4a が確かめる）．
- **これで draft の「説明できないこと」二つが説明できる**：v3（`source_out` を一度も呼ばない）が「源を先に」で動いたこと，2026-11 の bench で #2 の `test_pps_input` が通ったこと（`plan/phase8/bench-results.md` の「#1 → #2 の 10 MHz と PPS」）．なお電源投入時の FPGA の既定は `PPS_OUT_EN = 0`，PPS は external 選択（`fpga/usrp3/top/x300/bus_int.v:386-391` の `at_reset(7'b1000000)`）なので，「UHD で一度も開いていない #1」は PPS を出さない．bench の観測は「#1 を以前に UHD で開いたままの状態」だったと読むのが整合する（INFERRED）．
- **最小の列と観測**：S4（「S3 から #1 の `set_time_source_out(true)` を抜く → #2 に時刻が入らず確認が false」）を流す → **#2 に時刻が入り，確認は true**．draft の表では「期待と違う」ので，当日「S3 の手順か配線がおかしい」という誤った切り分けに進む．
- **本当に残る危険は別の所**：PPS OUT は「開いている途中」だけ切れる（構築の register 書き込みから `init_prop_tree` まで．LMK の初期化と lock 待ち 1 s を含む数秒，`x300_mb_controller.cpp:71-80`）．REF OUT も LMK の初期化し直しで途切れる（INFERRED）．v3.0.17 の「源を先に」は，**follower が lock した後に源を開く**と follower の参照が途切れることの回避だったと読める．B の「全部を `internal` で開いてから順に切り替える」はこの危険を避けているので，B の手順そのものは正しい．
- **直し方**：(a) D-1 の「(UHD turns PPS OUT off at each open)」と D-4 の「outputs REF OUT and PPS OUT after its open」の根拠を直す．`set_sources_out` を足すこと自体は「既定に頼らない明示」として残してよい（1 呼び出し）が，必須の理由は無くなる．足さないなら UR-2 の追加が一つ減る．(b) S4 を二つに分ける：S4a「`source_out` を呼ばずに PPS が届く（期待 true）」，S4b「#1 に `set_time_source_out(false)` を出すと #2 に時刻が入らず，Module の確認が**拒む**（期待 false）」．S4b は時刻の確認の negative control で，今の plan には確認が「外れを捕まえる」ことを見る段が一つも無い．(c) S5(a) は #1 を開く**間** #2 の `ref_locked` を 10 ms ごとに読む（開き終えた後だけ読むと，再 lock して見逃す）．

### M2. 2026-11-27 の bench は D-3・D-4 の「全装置で同じ tick」を決められない．`last_pps` の一致は秒の一致しか示さない

- **機構（VERIFIED，FPGA source）**：X300 は PPS をまず参照 clock（10 MHz，100 ns の格子）で取り込み，次に radio clock へ移す（`fpga/usrp3/lib/timing/pps_synchronizer.v`：「first synchronized to a common reference clock shared between multiple devices」「the async delay for pps_in must be constrained along with the clock delay」，`x300_core.v:259-263`）．内部 PPS は `ref_clk` の縁で作られる（`x300.v:468-477`）．daisy chain では #2 に PPS と 10 MHz が別々の cable で届くので，#2 が PPS を取り込む 10 MHz の縁は「PPS の経路の遅れ − REF の経路の遅れ」の符号で一つずれうる．ずれは **100 ns = 200 MHz で 20 tick** で，cable の長さの組で決まり，境目の近くでは open ごとに変わりうる（INFERRED：FPGA の pad の遅れと LMK の出力の遅れは読んでいない）．これに加えて，zero-delay の PLL（`x300_clock_ctrl.cpp:604-617`，200 MHz は `m10M_200M_ZDEL`．**184.32 MHz は `m10M_184_32M_NOZDEL` で位相が決まらない**）は #2 の radio clock を「cable の遅れだけ遅れた」参照に合わせるので，約 5 ns/m ≒ 1 tick/m の固定の差が残る．
- **最小の列と観測**：S3 を通す（`last_pps(0) == last_pps(1)`，`time_now` の差 < 10 ms）→ D-1・D-4 を「bench で確認済み」と記録 → 2 台の受信 stream を一つの `at` で始める → 共通の信号の到来が stream の間で 20 tick（1 MS/s なら 0.1 sample，200 MS/s なら 20 sample）ずれ，つなぎ直しや cable の交換で値が変わる．`last_pps` は各装置が**自分の取り込んだ縁**で latch した数なので，物理のずれが何 tick でも必ず `k × mcr` で一致する．S0〜S5 のどの段もこれを見ない．
- **帰結**：(a) 11-27 が決めるのは「順に開いて切り替えると lock し，秒が合う」（D-1 の手順）だけ．D-4 の `pps` epoch と利用者の契約 §4 の「その瞬間の tick は全装置で同じ」は S6（RF）まで INFERRED のまま，と記録に書く．(b) S6 は「一回測る」では足りない：**open し直しを 10 回以上**（AGENTS §9 の bench の決まり）で差の分布を取り，20 tick の跳びが出るかを見る．(c) S6 に USRP の送信は要らない（共通の信号は信号発生器か雑音源でよい）．ただし RX port に入る level の確認は owner のもので，ここで緩めない．今の配線は #1 の TX が splitter を駆動するので，それを使うなら送信の前の att の確認が要る．(d) 184.32 MHz の master clock は今は binds できない（RM-28 の `master_clock_hz` は 200000000 だけ，`design/07-radio-model.md:305`，`crates/ezsdr-radio/src/x3xx.rs:17`）ので今は問題にならない．後で足すときは zero-delay でない（NOZDEL）ことを束の揃えの前提として測り直す，と note に一行．(e) 差の置き場は既にある：channel ごとの `radio.d.c<n>.path_delay_samples`（RM-23）か TM-18 の関係の uncertainty．新しい概念は要らない．
- **S0 の役の入れ替えは cable の付け替えが要る**：draft は「#1 の `ref_locked` がまだ false なら #2 を源にして役を入れ替える」と書くが，REF OUT/PPS OUT と入力の cable を物理に付け替えないと成り立たない．当日の owner の作業として plan に書く．また #1 の `ref_locked` が false のまま源にすると，#1 の radio clock と REF OUT（LMK）が TCXO に lock しておらず，PPS（`ref_clk` から）と REF OUT が coherent でなくなる（INFERRED）．これは M4 の「`last_pps` の増分が `mcr` ちょうど」の確認で捕まる．
- **5 回は少ない**：KL-1 の lock 失敗が open あたり約 4 % なら，5 回では「通る」と「4 % で落ちる」を区別できない．各段 10 回以上に．
- 11-27 は Module の時刻合わせの code を一行も通らない（`sync_check.c` が手順を書き写す）．「D-1 は bench で確認」と書くときは「UHD の振る舞いを確認．Module の道は未試験」と分けて書く．

### M3. 喪失の帰属と健全な handle の解放：draft の「失った handle だけに印」は，今の code では装置を知る術が無く，取り違えたときの失敗は process の abort（F4）

- **証拠（VERIFIED）**：喪失を公表する口は `Core::device_lost(&self, message: &str)` 一つで，装置の識別を取らない（`core.rs:1076-1098`）．呼ぶ所は 10 か所（`control.rs:654, 665, 670`，`rx.rs:594, 614`，`tx.rs:1383`，`core.rs:1104, 1283, 1368`，`mod.rs:409`）．`Device::mark_lost(&self)` も引数が無い（`device.rs:223`，`uhd.rs:1147`）．だから draft §13.2 の「どの handle かは，`lost: true` を返した呼び出しの handle，読みが失敗し続けた timekeeper，1 s 黙った受信 stream でわかる」は，Provider の喪失の道を 10 か所とも変えるか，合成が「最後に `lost: true` を返した handle」を覚えるか，のどちらかを要る．前者は §5 の「Provider の装置の呼び出し約 66 か所は変わらない」と両立しない．後者は，link が死んだときの普通の現れ方（1 s の沈黙 `rx.rs:614`，時刻の読みの 1 s の失敗 `control.rs:670`．どちらも `lost: false` か error 無し）では handle が分からず，draft の規則で「全 handle に印」になる．つまり「失った handle だけ」が効くのは少数の場合だけ．
- **最小の列と観測（INFERRED：F4 の機構は design-notes §17 と `uhd.rs:671-697` のコメント，B への当てはめは推論）**：2 台が同じ switch につながる．switch が瞬断する → #2 の `rx_recv` が `lost: true` を返す → #2 だけに印 → Run が終わり合成が drop される → #1 の `UhdDevice::drop` は `answers()` を一度読む（`uhd.rs:638-642, 679-681`）．link が戻りかけで読みが通る → `close_streams` と `uhd_usrp_free`（`:693-696`）．teardown は register を書く．その途中で link がまた落ちる → UHD の destructor から例外 → **process が終わり，Manifest が書かれない**．N = 1 にもこの窓はある（S-B1）が，N = 1 で「印の無い装置が喪失の後に解放される」ことは無い．B の選択的な印は，喪失のたびに健全とみなした N−1 台をこの窓に通す．
- さらに，健全な handle は「continuous の受信を止めていない，送信の burst を EOB で閉じていない」まま解放される（§3.4：喪失の後はどの装置も呼ばない）．N = 1 には無い新しい teardown の状態で，実機で一度も通っていない．
- **server では得るものが無い（VERIFIED）**：`ezsdr-server` は一 process で一つの Session だけを扱い（`crates/ezsdr-server/src/main.rs:1`，client は Session ごとに `Popen`，`python/ezsdr/_client.py:70`），本体で UHD を開くのは `crates/ezsdr-server/src/catalogue.rs:29, 144` と公開の helper `crates/ezsdr-radio-uhd/src/lib.rs:66` だけ（`grep -rn 'open_uhd\|UhdDevice::open' crates --include=*.rs`，tests を除く）．process が終われば handle は OS が片付ける．`reclaim`（`uhd.rs:647-668`）が効くのは同じ process での次の open だけで，server にはそれが無い．library として同じ process で開き直す利用（bench の道具など）では，保持が process の終わりまで続く点だけが違う．「喪失の後に健全な装置を解放できる」は draft が B の利点四つの一つに数えているが，利用者に見える効果は無い．
- **より良い案**：**喪失のとき全 handle に印を付ける**（合成の `mark_lost` は全 handle に送るだけ）．`mark_lost` に識別は要らず，Provider の 10 か所は変わらず，新しい teardown の状態も無い．どの装置かは D-5 の記録（message と `timing` の行）のためだけに「分かれば書く，分からなければ `unknown`」とする．取り違えても行の名前が違うだけで，process は落ちない．KL-2 の文は「どの装置の喪失でも，全装置の handle が process の終わりまで保持される」になる．draft §13.3 の B の利点から「健全な装置の解放」を外す（B を選ぶ理由は fake・mutex・異種の道の三つで足りる）．
- **D-5 との関係**：「`DEVICE_LOST` の message が装置を名指す」（owner の決定）は，上の事情で**常には守れない**（沈黙と読みの失敗では stream か timekeeper から引けるが，panic `mod.rs:409` では引けない）．D-5 の文を「分かるときは名指す」に直すかは owner の判断（利用者に見える）．

### M4. Module の時刻合わせ（§13.2）：確認の読みが縁をまたぐと健全な束を拒み，一回の再送で一台が 1 s ずれ，拒むだけで終わる．N = 1 と道が二つになる

- **証拠（VERIFIED）**：C API に `uhd_usrp_get_time_last_pps`・`uhd_usrp_set_time_next_pps` はある（`usrp_c.cpp:368, 384`）．X300 の control は UDP の一往復で，応答が無いと **1.0 s 待って最大 3 回**やり直す（`x300_fw_ctrl.cpp:29, 44-57, 62-75, 124, 159`）．
- **列 1（一台だけ次の秒）**：handle 0 の縁を見る → handle 0 に `set_time_next_pps(0)` → handle 1 への書き込みの UDP が一つ落ちる → 1.0 s 後に再送 → handle 1 は次の縁を越えてから arm される → handle 1 の時刻は handle 0 より 1 s 遅れる．draft の確認（`last_pps` の一致）はこれを捕まえる（ここは良い）．しかし draft は「外れていれば拒む」だけなので，利用者には「つなぎ直せ」としか言えない．KL-1（lock 失敗 1 − 0.96^N）と合わせて connect の失敗率が上がる．**直し方**：外れたら手順を最初からやり直す（最大 3 回など．`set_time_next_pps` は何度でも出せる）．それでも外れたら拒む．
- **列 2（偽の拒否）**：確認で `last_pps(0)` を t = 0.9998 s に，`last_pps(1)` を 1.0003 s に読む（間に縁）→ 揃っているのに 1 秒違って見えて拒む．縁の直後に読めば稀だが，host の preemption か上の 1 s の再送で起きる．**直し方**：UR-8 の bracket と同じ形にする．handle 0 → 全 handle → handle 0 の順に読み，前後の handle 0 の値が同じときだけ比べる．違えば読み直す．
- **確認を少し強くできる（INFERRED．bench で先に試す）**：時刻を 0 に入れた後，各 handle の `last_pps` は縁ごとに `mcr` ずつ増えるはずである（10 MHz と PPS が coherent なら）．二つ三つの縁で `last_pps` の増分を見れば，(i) PPS が届いていない装置，(ii) 共有の PPS を受けながら**自走している別の参照**に lock した装置や lock していない装置，(iii) 10 MHz と coherent でない PPS，を connect のときに捕まえられる．**限界**：PPS の取り込みは 100 ns の格子（M2）なので，2〜3 s の観測で見えるのはおよそ 0.05〜0.1 ppm 以上のずれ（1 縁で 20 tick の跳び）だけである．TCXO の自走（ppm の桁）は捕まるが，別々に規律された二つの参照（GPSDO どうしなど）は捕まらない．また増分は取り込みの揺れで ±20 tick 動きうるので，判定は「`mcr` ちょうど」ではなく幅つきにする．draft の「別の 10 MHz の装置は捕まえない」は，今の確認（`last_pps` の一致と 10 ms）についてはそのとおりで，この増分の確認を足せば自走の場合だけ捕まる．`sync_check` の S3 に「3 つの縁の `last_pps` の増分を各 handle で印字する」を足せば，11-27 に実際の揺れが分かり，owner の判断（§7 の 6）の前に効くかどうかが決まる．これは UR-7 の `Pps` epoch の約束（「every whole second of the root is a PPS edge」，`design/18-uhd-radio.md:161`．今は INFERRED と書いてある）を，宣言ではなく connect のときの確認にする．費用は読みが数回と 2〜3 s．
- **道が二つ**：draft は N = 1 を今の `set_time_unknown_pps`，N ≥ 2 を Module の手順にする（§13.2）．合成も N = 1 では使わない（§3.8）．自分の原則 6（「N = 1 だけの別の道は作らない」）に反する．N ≥ 2 の道は 2 台の bench でしか通らず，一つの規則が二か所に実装される形（AGENTS §6 の three strikes の問い「is a rule implemented twice」）になる．**直し方**：外部 PPS の時刻設定は N によらず Module の手順一つにし（N = 1 は handle が一つの同じ loop．確認も N = 1 に付く），`uhd_usrp_set_time_unknown_pps` は使わなくなる．合成も N = 1 で通す（写しは index 一つ．hot path の費用は `dyn` の一段で，測って決めればよい）．N = 1 の振る舞いを変えない保証が欲しいなら，合成を通した N = 1 を今の differential の rig に載せるほうが，道を分けるより強い．orchestrator の判断の範囲だが，draft の原則と食い違うので挙げる．
- **fake（UR-33）**：fake の `set_time_next_pps` は「共有の手動の時計の次の整数秒まで時刻が変わらない」ことを本当に模すこと．列 1 を書ける fault（「この handle の arm は一秒遅れる」）が無いと，やり直しと拒否の道がテストできない．待ちは host の時計の sleep にする（手動の時計は全員が idle のとき進む，`host.rs` の M2・M5）ので，1 ms の busy poll にしない．

## 3. Minor・note

### m0. 「葉の trait は変えない，既定の本体つきの `timekeeper(i) -> &dyn Device`」は，そのままでは書けず，書けても wrapper が束を黙って隠す（orchestrator の判断の範囲．実行時の列は示せないので minor）

- **書けない（VERIFIED：Rust の規則）**：trait の既定の本体で `self` を `&dyn Device` として返すには `Self: Sized` が要り，その束縛を付けると `dyn Device` から呼べない．だから「今の wrapper をそのまま通す」（§3.9，§13.2）は成り立たず，十数個の `impl Device` に一行ずつ足すことになる．
- **書けても危ない**：既定が「自分は一台」だと，合成を包む wrapper（`provider/test_support.rs` や `tests/common/` の probe の類）が転送を忘れたとき，束が N = 1 に見えて，テストは別の道を通ったまま通る．これはこの crate で既に一度起きた型である：`device.rs:230-236` の `real_host()` は「a wrapper that forgets to forward `host()`」のために後から足された．
- **直し方（どちらか）**：(a) `fn timekeeper(&self, i: usize) -> Option<&dyn Device> { None }` を既定にし，`None` は「束ではない」を意味させ，自由関数 `tk(d, i)` が `d.timekeeper(i).unwrap_or(d)` を返す．既存の wrapper は変えずに済む．合成を包む wrapper は「転送しないと `timekeepers() > 1` の rig のテストが落ちる」ように rig を組む．(b) 束の事実（serial，source，rate の照合，時刻の確認）を `Device` に通さず，合成の型（`Bundle`）を Authority と `from_binding` に**具体の型で**渡す．Provider の hot path は `Arc<dyn Device>` のまま，装置ごとの事実は `Bundle` の method になり，trait に番号が漏れない．(b) のほうが「handle の形は `Device` の下に隠す」（原則 1）に素直で，UR-2 の「exactly」の list の変更が最小になる（推奨）．どちらも orchestrator の判断の範囲．

- **m1. 一つの gate を握ったままの呼び出しは「数 ms 以下」ではない**（§3.3 の INFERRED）．死にかけの handle では一回の control の呼び出しが 1〜3 s 塞がる（`x300_fw_ctrl.cpp` の 1.0 s × 3．stream command の経路の timeout は読んでいない）．列：#2 の link が落ちた直後（未検出）に radio への `Stop` → #2 の受信の持ち主が gate の下で `rx_stop` に塞がる（`core.rs:1560`）→ #1 の持ち主が gate を待つ → D-6 の共有の 1 s を全受信が越え，健全な #1 の持ち主まで detached になり，`close_streams` を飛ばして leak を記録する．D-2 のもとでは結果（radio の喪失）は同じなので major にしないが，§3.3 の文は直し，D-6 の KL の文に「一台が塞がると全装置の stream が detached になりうる」を入れる．gate を装置ごとにする必要は無い．
- **m2. `Device` の単数の method の合成の意味を決める**：`master_clock_rate`（束の中で違えば拒む．`to_time_spec` は handle の `mcr` を使う，`uhd.rs:597`），`time_now`（handle 0），`ref_locked`（全部の AND か装置ごとか），`describe`，`fidelity`．draft は `fidelity`・`describe`・`host` しか書いていない．
- **m3. `reopen_on_unlock` は束ごと開き直す**（`crates/ezsdr-server/src/catalogue.rs:133-165` の loop は radio と device を捨てて開き直す）．B でも「全 handle を捨てて全部を `internal` で開き直す」こと．lock しなかった follower だけ開き直す最適化は安全だが，源だけ・一部だけの開き直しは M1 の途切れを起こす．draft §7 は「compound ごと」と A の言葉のまま．
- **m4. 同じ装置を二度書いた list**（X310 の二つの SFP の address）：serial の照合（UR-5）は open の後なので，二つ目の open が一つ目の handle の下の装置を初期化し直してから拒むことになる（UHD の claim は同じ process を通す，`x300_claim.cpp`）．拒否は正しく出るが，`uhd_usrp_find` で serial を先に引いて open の前に拒むほうが安い．
- **m5. 別の Session に分けた装置が daisy chain で参照を分け合う構成**（D-4 の「mixed setups は別の Run」）：源の装置を持つ Session を開く・開き直すたびに，もう一方の Session の装置の参照が途切れる（M1 の機構，INFERRED）．KL に「別々の Run が参照を共有するなら分配器を使うか，源の Session を先に開いて最後まで保つ」を一行．S5(a) が証拠になる．
- **m6. connect の時間**：open は一台ずつ直列（C API の `uhd_usrp_make` は一つの mutex，`usrp_c.cpp:215-218`）で，follower の切り替えは最大 30 s の lock 待ち（`x300_mb_controller.cpp:236-255`）．N 台で connect が N 倍近く伸びる．利用者に見えるので KL か §4 に一行．
- **m7. draft の行番号は `e008038b`**．`ea7bc5c1` で確かめた範囲（`core.rs:1076-1098, 1553-1567`，`control.rs:645-674`，`uhd.rs:621-697`）は合っていた．
- **確認して異議の無い点**：compound の並列の初期化（`x300_impl.cpp:198-216`），`set_time_unknown_pps` が警告だけであること（`multi_usrp_rfnoc.cpp:909-946`），D-6 の算術（1 + n_tx + n_rx 秒）と「段ごとに共有する 1 s」，uhd-control が booking で装置を呼ばないこと（`control.rs:234-644` に `device.` の行は無い）．§12 の「Kernel を変えない」は draft の読みを受け入れただけで，自分では確かめていない（§8）．受信が overflow で自分から止まること（`radio_rx_core.v:586-595` の overrun → error 報告）は妥当に読めるが，flow control が radio まで背圧を返すことは読んでいないので INFERRED のまま．

## 4. この決定が後で閉じるもの（利用者が要りそうなもの）

- **8c（Replay を Provider の中に）**：D-2 の「以後どの装置にも呼び出しをしない」は，host が供給する送信が underflow で黙ることに頼っている．Replay のように装置の中に源がある送信は，健全な装置で**出続ける**（RF の安全）．D-2 の設計 note に「装置の中に源を持つ送信を足すとき，喪失の公表の前に健全な handle へ stop を出す道を足す」を，D-2 が既に約束している「装置ごとの喪失を閉じない」と並べて書く．M3 の「全 handle に印」は解放の話で，これと矛盾しない（印は「解放しない」，呼ぶかどうかは Provider の `lost`）．B は装置ごとに graph が一つなので Replay 自体は閉じない（draft と同じ．INFERRED：C API は `multi_usrp` だけなので 8c は C++ の層が要り，それは B でも A でも同じ）．
- **HackRF・bladeRF の Module**：B の「時刻合わせは Module が handle をまたいでする」は，compound を持たない driver にそのまま移る形で，A より一般性がある．`args` が list であることは UHD Module の selector の話で，語彙には漏れない．閉じるものは無い（INFERRED：code が無い）．
- **同期していない radio は別の Run**：閉じるのは「一方の受信で他方を低い遅れで動かす」道だけ（draft §12(2) のとおり）．m5 の参照の途切れを KL に書けば足りる．
- **模擬の複数 radio**：閉じるものは無い．ただし MockRadio の「stream は装置をまたがない」（D-3）は，RM-28 の timekeeper を外す段で Mock にも同じ拒否を入れないと「Mock が実機より緩い」（§3 の Mock の原則）になる．draft §11 が後に回しているので，その段の brief に入れる．
- **装置をまたぐ位相の再現**（O11）：`aligned` は時刻の揃えで，LO の位相は timed tune が無いと open ごと・tune ごとに変わる（RM-9 の `random_unless_timed_tune`）．④ の範囲外だが，§4 の「できないこと」に一行あると，束を array として使う利用者の期待と合う．

## 5. 代わりの案（方向について）

方向（B）より良い全体案は無い．A は M1 の後でも「並列の初期化が daisy chain で安全か」が未確認で，fake の費用が大きい．G は二つの書き方を今払う理由が無い．**B の中で変えるべきは三つ**：喪失は全 handle に印（M3），外部 PPS の時刻設定は N によらず一つの道で，やり直しと `mcr` の倍数の確認つき（M4），束の事実は trait ではなく具体の型で渡す（m0 の (b)）．三つとも draft より code が減るか同じで，Provider は draft の言うとおり本当に変わらなくなる．

## 6. Verdict

**B は可．draft は M1〜M4 を直してから brief と test plan に進む．** M1 と M2 は 2026-11-27 の前に bench plan（§8.1）を直す必要がある（S4 の期待値，S6 の位置づけ，cable の付け替え，回数）．M3 と M4 は設計の文の直しで，方向も Kernel も変えない．

## 7. owner に決めてもらうこと（利用者に見えるもの．一つずつ）

1. **D-1・D-4 の根拠の訂正を受け入れるか**（M1）：「UHD は開くたびに PPS OUT を切る」は誤りで，開いた X300 は PPS OUT を出す．推奨：D-1・D-4 の文を直し，源への `set_sources_out` は「既定に頼らない明示」として残すか外すかを orchestrator に任せる．B の選択は変わらない．
2. **「同じ tick」の約束をいつ言うか**（M2）：推奨は，11-27 の後は「秒が合う」までを確認済みとし，D-4 の `pps` epoch と tick の揃えは S6（RF，送信なしで可，open し直し 10 回以上）まで INFERRED と KL に書く．代わり：S6 を 11-27 に入れる（信号発生器か雑音源が当日あるなら）．
3. **喪失の後の handle**（M3，KL-2 の文）：推奨は「どの装置の喪失でも全装置の handle を process の終わりまで保持する」．代わり：draft の「失った handle だけ」（利点は見えず，取り違えで process が落ちる窓がある）．
4. **D-5 の「message が装置を名指す」**（M3）：推奨は「分かるときは名指す．分からないときは `unknown`」．代わり：喪失の道 10 か所に装置の識別を通して常に名指す（panic の場合だけは残る）．
5. **時刻合わせが外れたとき**（M4）：推奨は 3 回までやり直してから拒む．connect が最大で数秒伸びる．代わり：一回で拒む（draft）．
6. **connect のときの coherence の確認**（M4）：11-27 に S3 で増分の揺れを見てから決める．推奨は，揺れが幅に収まるなら `last_pps` の増分を 2〜3 縁で見る（connect が 2〜3 s 伸び，自走する別の参照と coherent でない PPS を拒める．N = 1 の外部 PPS にも付く）．代わり：付けない（draft．配線は利用者を信じる）．
7. **bench の当日の作業**（M2）：#1 の `ref_locked` が false のままなら cable を付け替えて役を入れ替える，各段 10 回．owner の手が要るので先に知らせる．

orchestrator が決めること（参考）：M4 の「道を一つに」，m0 の (a)/(b)，m1〜m7．

## 8. 確かめていないこと

- M1 は source の上では端から端まで読んだが，実機では実行していない（S4a）．v3 と bench の観測とは合う．
- FPGA の pad と LMK の出力の遅れ（M2 の 20 tick の跳びが今の cable で起きるか）．zero-delay の位相の再現性．
- stream command の経路（RFNoC の ctrlport）の timeout（m1 の 1〜3 s は ZPU の register の経路の値）．
- `UhdDevice::drop` がどこで走るか．`cleanup` は device を手放さない（`provider/mod.rs:773-790`）ので instance の drop のときと読めるが，それが RS-8a の step の中か外かは追っていない（死んだ handle の `answers()` は一台最大 3 s．M3 の「全 handle に印」なら `answers()` を呼ばないのでこの問いは消える）．
- §12 の事実のうち自分で確かめたのは「server は一 process 一 Session」と UHD を開く所だけ．Session の directory，環境変数，RS-19・SB-16・SC-23b の読みは draft のものを受け入れただけで，確かめていない．
- D-6 の「健全な stop は 0.2〜0.4 s」（draft の INFERRED）．
- 健全な handle を「止めていない stream つき」で解放したときの UHD の振る舞い（M3）．受信の flow control が radio を止めること（draft の INFERRED のまま）．
- `multi_usrp` から RFNoC の graph に届く道（8c）．二つの Session の同時の実走．
