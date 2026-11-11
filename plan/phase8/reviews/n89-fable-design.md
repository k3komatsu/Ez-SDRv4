# #89 design review (AGENTS §9 step 0) — Fable 5.2

- Reviewer: Fable 5.2 (`claude-fable-5-2`)，read-only．base は `main` = `8dd1016`．
- 対象: `sketch.md`（Owner decisions Q1–Q6 と §0–§11），`design-brief.md`，issue #89．
- tree では何も build / 実行 / 編集していない．probe は 1 本だけ scratchpad で実行した（付録 P1）．hardware には触れていない．
- VERIFIED = 引用行のコードを読んだ，または probe を実行した．INFERRED = 推論，または未実行．

## 判定: **the direction holds with changes**

R（Kernel `disconnect_signal()` + server の入力 thread + Python `start_new_session` / abandon）は成立する．owner 決定 Q1–Q6 の前提を覆す証拠は見つからなかった．

- lock 順序と wake の経路は，既存の data thread（KC-46a / KC-46b）がすでに通っている道と同じで，新しい deadlock は見つからない．
- ただし brief / test plan に入る前に直すべき点がある: major 1 件（F1），minor 6 件（F2–F7），note 5 件．
- 加えて，owner に判断してほしい代替案が 2 つある（A1，A2）．

---

## 1. コードと突き合わせて成立を確認したこと

| 確認項目 | 結果 | 根拠 |
|---|---|---|
| foreign thread からの `ending::request` の lock | `end` → `also` の順にしか取らず，control thread はどちらも待ちながら保持しない．data thread が同じ関数を既に呼んでいる | VERIFIED `ending.rs:210-226`，`paced.rs:70,76` |
| `paced::wake` と RS-6 step 1 の競合 | `wake_handle` を `schedule` の間保持するので，FreezeDispatch が cancel するか no-op が残るかのどちらか．KC-46a が「step 1 の後の wake は no-op として残る」と既に書いている | VERIFIED `paced.rs:153-173`，`ending.rs:281-282`，`design/06-kernel-coordinator.md:436` |
| lost wake（loop 先頭の `end` 検査と `next_wakeup` の間に signal） | 起きない．`schedule` が queue に入れてから generation を上げ，`next_wakeup` は generation を queue より先に読む | VERIFIED `authority.rs:284-310, 523-555`，`host.rs:355-372` |
| signal を作る時点で cause を固定してよいか | よい．Lease の mode は `adopt` / `renew` / `Release` で変わらない | VERIFIED `run.rs:412-440`，`pipeline.rs:975-990` |
| `connect` 中の signal | watch への登録は `coordinator::connect` の後なので，fired 済みなら即時に呼ばれ，`advance_to(t0)` が loop 先頭で返り，`ended` + Manifest になる | VERIFIED `lib.rs:225-240`，`stepping.rs:313` |
| Run が自分で終わった後 / handle drop 後の signal | `request` が false を返す，または `Weak::upgrade` が失敗する．どちらも no-op | VERIFIED `ending.rs:211-215` |
| client が出力を閉じた後の reply | `write_reply` が `BrokenPipe` で Err → `Drop for Server` → `disconnect` → Manifest | VERIFIED `transport.rs:78-79`，`lib.rs:395-401` |
| Simulation の決定性（Q2） | host 時刻で終わる Simulation Run は既にある（Detached Lease の expiry を毎 round `check_lease` が host clock で見る）．Q2 は新しい種類の非決定性を持ち込まない | VERIFIED `stepping.rs:349`，`mod.rs:262-267` |
| Detached の拒否（Q3） | server と Python に Detached の利用はなく，`lease` の mode を見るだけで足りる．`Adopt` / `Renew` は Attached では RS-24 で rejected になるので穴は残らない | VERIFIED `run.rs:432-440`（sketch 付録 A の検索も再確認） |
| KC-33 / KC-32 / RS-23 / RS-25 / MA-30 | 文面は変更後も真 | VERIFIED `design/06:384,389`，`design/04:192,196` |

---

## 2. Findings

### F1（major，VERIFIED）— test 方針（sketch §9）では，wake を消した mutant を確実には殺せない

sketch §9 は「signal が call より先でも verdict は変わらない」と書く．それがそのまま弱点になる．

**反例（Kernel test `kc_36_a_disconnect_signal_ends_a_waiting_advance`）:**
```
mutant: disconnect_signal の closure から paced::wake(&shared, true) を消す   // = #89 そのもの
thread A: run.advance_to(T0 + 1 h)
thread B: signal()
B が A の next_wakeup 進入より先に走る:
  A: schedule → run_loop 先頭で end を見て return（stepping.rs:313）→ settle → Err(Ended{client_disconnect})
→ test は green．
```
- thread の起動順しだいで，kill が出たり出なかったりする．
- server test `ea_15_an_input_that_ends_during_an_advance_…` も同じ．frame を書いてすぐ `shutdown(Write)` すると，入力 thread は `advance` frame と EOF を続けて読む．control thread が frame を取る前に fire する公算が高く，wait の中の cut を一度も通らない．
- data thread が event を 1 つでも deliver すると条件付き wake が出る（`paced.rs:75-76`）．これも wake 抜けを隠す．

**直し方:**
- Kernel test は「control thread が `next_wakeup` で park した」ことを test Authority の probe で観測してから fire する．
  - `WallAuthority` は test support（`crates/ezsdr-kernel/tests/support/paced.rs:130`）なので足せる．
  - sleep や `recv_timeout(150 ms)` で代用しない．
  - Run は event を出さない構成にする．
- UHD Authority 側の「foreign thread の schedule が待ちを切る」は既存 test に任せる（`ur_07_schedule_wakes_a_waiting_next_wakeup`，`plan/phase7/exit-review/18.md:13`）．
- server test は配線だけを pin する（入力 thread が fire する，child の登録）．
  - 「wait 中」は server crate から観測できない．`HostClock::blocked()` は `pub(crate)`（`host.rs:487`）．
  - test plan の各行に「この test が殺す mutant」を書く．wake の行を server test に割り当てない．

### F2（minor．ただし KC-36 の draft 文が偽）— fire の後に読まれた frame が処理され，`submit` は dispatch まで進む

`submit` の前置きは `check_lease` と `ensure_live` で，後者は `CleanedUp` しか見ない（`mod.rs:231-238`）．admission が見るのは `frozen` だけ（`admission.rs:38-40`）．`end` が要求済みで cleanup 前の Run は，Action を admit して Provider の queue に push し，その後に settle する（`pipeline.rs:1033-1048`）．

**系列:**
```
Python: sdr.tx.repeat(wave) の stdin.write() 完了後，flush() の前に Ctrl-C
  → abandon: stdin.close()   // BufferedWriter は完全な frame を flush してから EOF（probe P1，VERIFIED）
server 入力 thread: frame を転送 → EOF → fire（end = client_disconnect）
control thread: frame を取る → submit → admit → dispatch → wait_finished は即 return（paced.rs:186）→ settle
```
- 観測: client が去った後に `start_repeat` が Admitted で log に載る．
- Provider thread が freeze より先に取れば，送信が stop まで（約 10 ms から S）出る．送信が実際に出るかは競合なので INFERRED．
- 上界（D + W + S）は破らない．今日の挙動（frame を処理してから EOF）とも同じで，regression ではない．
- しかし次の 2 つと合わない．
  - sketch §2.3 の「pipe を閉じることが departure」．
  - KC-36 draft の「cleanup は … or the next call does」．次の call が `submit` なら，cleanup の前に dispatch する．

**直し方（lean な順）:**
1. server: fire 済みなら次の frame を `handle` せず，EOF と同じ道へ行く（framing loop に 2 行）．「入力が終わった後は request を処理しない」という規則になり，§8 の pipelining の但し書きも消える．
2. または Kernel: 各 `RunHandle` call の入口で，pending の end を settle する．data thread が要求した end にも効く．ただし「frozen で Rejected の entry が残る」という既存の挙動が `Err(Ended)` に変わるので，既存 test に触れる．

推奨は 1．frame を取った後に EOF が来る競合は原理的に残る．契約表には「departure の前に送り切った request は処理されうる」と書く．

### F3（minor，VERIFIED）— 契約表「In `finish`」列の `Stopped { client }` は保証できない

`finish` frame の直後に入力が終わると，signal の `request` と `ending::finish` の `request`（`ending.rs:197-202`）は先着順になる（KC-32）．

- 起きる場面: client が `finish` を書いてすぐ死ぬ．`finish` の reply 待ちで Ctrl-C して abandon する．
- 観測: Manifest（`finished` reply の中身も）が `client_disconnect` になりうる．
- 直し方: 表と §10 の系列 5，10 を「`client` か `client_disconnect`（先着）」にする．test は Kernel level で順序を固定して assert する．

### F4（minor，INFERRED．Q4 のコストとして sketch に無い）— 止まった server を Ctrl-C で殺せなくなる

- 今日: terminal の Ctrl-C は server も殺す．cleanup は無いが，process は必ず消える．
- 変更後: server は別 session になる．control thread が cut できない所で止まっていると，次のようになる．
  - Python は 30 s 待って `kill()` する（`_client.py:106-109`）．
  - その待ちの間に 2 回目の Ctrl-C を押すと，Python だけが終わり，server は残る．
  - ssh wrapper 経由では，`kill()` が殺すのは local の ssh だけ．remote の server は残る（これは今日も同じ）．
- 止まりうる場所の例: `bounded` の外の Module call．読まれない stdout への write．
- 観測: client がいないのに送信が続く．手で `kill` するまで終わらない．
- 具体的に hang する経路は見つけていない．cleanup 自体は RS-8a で有界．だから INFERRED とした．
- ただし #89 の目的は「送信が client より長生きしない」ことなので，最後の網が無くなる点は owner に見せるべき．代替案 A1 がこれを塞ぐ．

### F5（minor）— spec の変更一覧（§6.2）の抜けと不整合

- **EA-15** の現行文は「a reply it cannot write … finishes it as a disconnect does」（`design/16-easy-api.md:81`）．`BrokenPipe` で exit 0 にするなら，exit status も EA-15 の文に入れる．§2.2 には書いてあるが，§6.2 の一覧に無い．
- **EA-15 の第 2 文**「When its input ends before `finish`」は，binary では「先読みした frame が残っていても」という意味に変わる．`ezsdr-server < script` は `connect` の直後に切れる．
  - repo 内に pipe で script を流す利用は無い．`ea_binary_speaks_the_protocol` は対話式（`protocol.rs:1223-1241`）．
  - 検索: `grep -rn "ezsdr-server\b" --include=*.sh --include=*.py --include=*.md --include=*.rs`
  - draft の「`serve` は test 用」だけでなく，binary の意味をここに 1 文で書く．F2 の直し方 1 を採れば「入力が終わった後は request を処理しない」と書ける．
- **KC-36 draft** の「the `RunHandle` call in progress … or the next call does」は F2 のとおり不正確．
- **KL-43** に足す項目:
  - (a) client の pipe を継いだ process（`os.fork`，multiprocessing の fork worker）が生きている間は EOF にならない（INFERRED）．
  - (b) abandon の後 30 s で server を kill する．その場合は Manifest が無い．
  - (c) F4．
- **RS-42** の文面は真のまま．Manifest の `deterministic: true`（`ending.rs:86`）は host 時刻で切られた Session にも付くが，Lease expiry という前例がある．変更は不要．
- 他に偽になる rule は見つからなかった．確認した rule: EA-2，EA-5，EA-11，EA-14，KC-21a，KC-29，KC-33，KC-46b，KC-46c，MA-29，MA-30，RS-23，RS-25．

### F6（minor）— Python abandon の細部（brief に書くべきこと）

- `try` は `stdin.write` を含める．書きかけの中断も abandon にする．`_client.py:83-87` の `BrokenPipeError` 変換は残す．
- abandon 後の `close()` は何度呼んでも同じ結果にする．
  - 表の「`finish` 中の SIGINT → `manifest` は None」は，同じ host なら 2 回目の `close()` で読める．
  - 規則は 1 つにする:「abandon 後の `close()` は exit を待ち，読めれば読む」．
- 30 s の kill の後は `manifest.json` が無いか，書きかけでありうる．読めない，または JSON でないときは None にする．例外にしない．
- `connect()` 中の Ctrl-C は，server が `connect` を終えて cleanup するまで（最大 30 s）prompt が返らない．今日は即死する．EA-16 に 1 句入れる．

### F7（minor）— bench（Q6）

- `8dd1016` での FAIL は，「drop の時点で `sdr.sleep` の残りが 15 s を十分に超える」ときだけ意味がある．
  - 既定は hold 120 s，settle 5 s（`ssh-drop-driver.py:19`，`ssh-drop-tx.sh:164`）で足りる．
  - script が残り時間を検査し，足りなければ INCONCLUSIVE にする．
- 旧 commit の FAIL run は，verdict の後に `docker stop -t 2` で server を止める（`ssh-drop-tx.sh:285-287`）．
  - これは PID 1 への SIGTERM，その後 SIGKILL．つまり E9 の条件（host の unclean death）そのもの．
  - この後も独立 receiver を読み続ければ，Q6(a) の最初の data point が追加の run なしで取れる．
  - (a) を別に流す場合も，同じ読み方を使える．

### Notes

- **N1（INFERRED）** signal の closure は `Weak::upgrade` で一時的に `Arc<Shared>` を持つ．
  - 入力 thread が最後の holder になると，device の drop が入力 thread で走る．process exit と競合しうる．Phase 7 Review Q が挙げた型（`plan/phase7/reviews/review-q.md:74-78`）．
  - 窓は µs，cleanup は 100 ms 以上なので，実際にはまず起きない．
  - ただで塞ぐ方法: `fire()` は watch の mutex の下で signal を呼ぶ．`Server::disconnect` / `finish` は Run を finish する前に同じ mutex を取り，watch を空にする．
- **N2** wake の条件 `shared.device_paced()` は，Phase 10 の RealtimeEmulation（WallPaced）を含まない．今は KC-2 が拒否している（`pipeline.rs:295-296`）．KC-36 の文は「host 時間で待つ class」と書いておくと後で漏れない．
- **N3** 入力 thread の channel は unbounded でなければならない．bounded にすると，EOF が backpressure の後ろに隠れる．EA-2 を破る client は server の memory を使える．`body_bytes` に上限が無いのは既知（`transport.rs:59`）．
- **N4** 将来 Detached を受ける frontend では，signal が何もしないので，長い request の間 TTL が始まらない．#89 と同じ穴．§8 に 1 行入れる．
- **N5** E5（`Provider::stop` の実測時間）は，raw archive の Manifest の `timing { what: stop }` を brief の前に読む．安い確認なので済ませる．

---

## 3. 代替案

### A1（owner 判断を勧める）— server 側の fail-stop backstop

**内容:** 入力が終わってから N 秒たっても process が残っていたら exit する．

- `main.rs` だけに入れる．watch の fire の後で timer thread が `process::exit`．約 8 行．library の `serve_watched` には入れない．
- N は RS-8a の最悪値より長くする．例: 60 s．
- **変わること:** 「client が見える形で去れば，server process が生きている限り D + N で必ず送信が終わる」と無条件に言える．F4 が消える．remote の server が残る件も消える．KL-43 が短くなる．
- **コスト:** backstop が働いたときは EOB も Manifest も無い（G と同じ）．停止は starvation 頼みで，E9 に依存する．Q6(a) がちょうどそれを測る．Q6(a) の結果を見てから採否を決めるのがよい．
- **Q5 との関係:** Q5 は「signal handler も heartbeat も無し」．A1 はどちらでもない．Q5 と矛盾しない．

### A2（推奨）— 「入力が終わった後は request を処理しない」

F2 の直し方 1．server に 2 行．F2 と F5 の EA-15 の曖昧さを消し，§8 の pipelining の但し書きも消える．Kernel は変えない．

### A3（任意）— child への伝播を Kernel に置く

**内容:** `pipeline::run_child` が走行中の child の `Weak<Shared>` を親の `Shared` に置く．親の signal が child にも `request` する．

- **変わること:** server の watch は「signal 1 個 + fired flag」になる．`drive` closure での登録と解除が消える．RS-25 の約束を，書いてある場所（Kernel）が守る．どの frontend にも効く．
- **コスト:** `Shared` に field 1 つと数行．「child を置いた直後に親の end を見直す」競合の処理が 1 つ．
- child は Simulation だけ（KC-37a）なので，急がない．sketch の sticky list のままでも正しい．

### 検討して退けたもの

- **`start_new_session` の代わりに `preexec_fn` で SIGINT を無視させる**: server のコードは要らない．しかし SIGHUP / SIGQUIT を覆わない．`preexec_fn` は thread-unsafe．Q4 の選択が良い．
- **EOF を poll で見る（入力 thread 無し）**: `unsafe` か依存が要る．却下．
- **Python の stdin を unbuffered にして flush の窓を消す**: partial write の loop が要る．A2 の方が安い．

---

## 4. Risk class と staging

- **risky** で正しい（lifecycle，thread 間の end）．S1 → S2 → S3 の順も妥当．
- S1 の test plan に F1 を入れる．park を観測してから fire する．wake の mutant は Kernel test に割り当てる．
- S2 に A2 と N1 を入れる．EA-15 の文は F5 を反映する．
- S3 に F6 を入れる．KL-43 は F5 の 3 点を足す．A1 は Q6(a) の後に決める．
- bench は F7 を反映する．旧 commit の run で E9 も読む．

## 5. owner に戻す点（1 つずつ）

1. **A2** を採るか．推奨: 採る．
2. **A1** を採るか．推奨: Q6(a) の結果を見て決める．それまでは F4 を KL-43 に書く．
3. **F3**: `finish` と departure が重なったときの cause は「先着」と書いてよいか．推奨: よい．
4. **A3**: 今は見送りでよいか．推奨: 見送る．

## 付録 P1（probe，scratchpad）

`p1.py`: Popen（`start_new_session=True`）の stdin に完全な frame を `write` し，`flush` の前に `KeyboardInterrupt` を起こす．abandon として stdin と stdout を `close()` する．

```
server got 28 bytes: b'{"request":{"op":"submit"}}\n'
abandoned after KeyboardInterrupt
```

→ abandon の `close()` は，buffer に残った完全な frame を server へ送ってから EOF にする（F2）．

## 確認していないこと

- `Provider::stop`（`provider/mod.rs`）の中身は読んでいない．cut の後の cleanup は `finish` と同じ経路だという理由で，既存の UR-16 / UR-26 に依った．
- docker と sshd の EOF 伝播は，bench (i) の idle の結果に依る INFERRED．
- Jupyter の interrupt（E12）は未確認のまま．
- cost: この review の token 数は手元で測れない．orchestrator 側の記録を使ってほしい．
