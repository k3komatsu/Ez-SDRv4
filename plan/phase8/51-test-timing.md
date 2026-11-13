# 51 — スレッドの走る時点で判定が変わるテストを型ごと止める（`mech:wall-clock-test-oracle`）

**決定済み**（owner 2026-11-11〜12：Q1〜Q17（Q10・Q15 は §5.3b で前提が変わった）。(i) は 2026-11-13 に取り消し，末尾の付録）。下は Fable 5.2 の v3.1 に orchestrator の訂正（§5.3a）を加えたもの．v1〜v3 と各レビュー（Sol 2 回，Opus 2 回）は `~/.cache/ezsdr-fixes/2026-11-11-test-timing/` にあり，段階を閉じる記録で `reviews/` に移す．段階 0（`51-tables.md`）と M（記録 `reviews/n51m-*`，#91 を閉じる）は済（2026-11-12）．1a（記録 `reviews/n51-1a-*`）も 1b（記録 `reviews/n51-1b-*`）も済．1c も済（記録 `reviews/n51-1c-*`，下の「段 1c の後」）．1d も済（記録 `reviews/n51-1d-*`，下の「段 1d の後」）．2a は作業中（`work/n51-2a`，下の「段 2a の後」）．

| 項目 | 内容 |
|---|---|
| 状態 | (v3.1) 下書き v3.1（AGENTS.md §9 step 0）。**この文書だけで読める**（v1 `sketch.md`，v2 `sketch-v2.md`，v3 `sketch-v3.md` は履歴で，参照しなくてよい）。v3 は Sol の v2 再 review（`sol-v2.md`，FAIL，major 2，minor 2）と owner の判断 (i) を受けた改訂。v3.1 は v3 の写しに，Opus の再確認（`recheck-v3.md`：PASS WITH CHANGES，major M1・M2，minor m1〜m7）の直しだけを入れたもの。変えた段落には「(v3.1)」をつけた。それ以外は v3 のまま |
| 書いた model | Fable 5.2（`claude-fable-5-2`） |
| 調べた時点 | `fe86629`。`parity.rs` は `git show fe86629:crates/ezsdr-acceptance/tests/parity.rs` の行番号（`parity.rs@fe8`）。tree は読むだけ。(v3.1) 「(v3.1)」の段落が新しく引く行番号は `1371ffa`（今の main）のもの。v3 のままの段落の行番号は `fe86629` のままで，振り直していない。ずれた所：`control.rs:484`→`:491`（受け取りの読み），`:591`→`:600`（`selected_at`）。`parity.rs` は #96（`aa478bc`）で変わった（`wait_for_events` は今も `:444-455`） |
| 証拠 | コードは `path:line`。probe 1〜6 は sim03 の rdev，リポジトリの外の写し（`probe/`）。**probe 7 は別物**：リポジトリのコードを含まない 20 行の crate を usrp-lnx03 で流した（cargo 1.99.1。`probe/probe7.rs`，`probe7.log`）。libtest と Cargo の挙動だけを見たもので，mutation の image での挙動は §6.4 の試験が通るまで INFERRED |
| 決まっていること | Q1 = A，Q2 = `Provider::join` を `host.wait` に，Q3 = (c)，Q4 = #96 を先に閉じる（(v3.1) **済み**：`aa478bc`。8 本を手順で直し，14 本をこの設計に預けた：§7.1），Q5 = Kernel の 12 本を含める，Q6 = 待ちの分類を先に，Sol major 4（mutation の正直さ）は #91 と一緒に入れる，**(i) は取り消し**（owner 2026-11-13，§5.3b）：予行（fake）は，時刻の遅れの証拠（§5.3b の 2 つ）がある trial を判定せず，印 `EZSDR-TIMING:` を書いて単独で流し直す。実機は変えない（遅れは今どおり INCONCLUSIVE） |

## 0. 結論と，v2 から変わったこと

**結論（変わらない）**：rig を駆動するスレッドを手動の時計の参加者にする（A）。Provider 直結の rig では，試験のコードがどれだけ遅くても device の時刻は動かない。Run を通す rig は A では閉じないので，窓に依存する場面を直結の rig へ移し，移せない実機の予行は囲う。mutation は「落ちた＝killed」をやめる。

**v2 から変わったこと**

1. **上限の分類を `finish()` の前にも置く**（Sol v2 major 1）。Run の rig の包みは，生成と**全部の method の戻り**で Run の終わり方を調べ，実時間の上限で終わっていたら，試験に制御を返す前に marker を出して 1 回だけ panic する（§6.2）。v2 の「`finish` で翻訳する」は生成と途中の assert を通らなかった。
2. **予行の判定の差し控えを，遅れの証拠だけで決める**（owner 2026-11-13，§5.3b）。証拠は，`TIME_ERROR` の cause が late（`late_at_device` を除く）／Kernel の log の時刻が窓を過ぎている，の 2 つ。差し控えるだけで，判定は作らない。受け取りの後の dispatch での遅れも RM-15 が許す host の遅れなので証拠に入る。`late_arrival_verdict`（`tests/common/iii.rs`）は作り直さず，実機の guard と N7 の判定として今のまま（判定の名前だけを引数に取る `late_arrival_verdict_of(what, …)` に分け，`late_arrival_verdict` は `"C"` を渡す。Session の Stop の N2 自身の late の drop も `"N2"` で通す：fix 1）。N7（わざと遅く受け取らせる 2 case）も述語を通す：C 自身の期待された late の drop と deadline だけを除いた `Lateness`（`iii::without_expected_late`）で `Rehearsal::trial` に入れ，C だけが遅い run は判定し，他の Action に遅れの証拠があれば判定せず印を出す。（v3.1 の「受け取りの記録だけで決める」（Sol v2 major 2，owner (i)）は取り消し。）
3. **Q10 は取り下げ**：2c は Manifest を変えない（`receipts` を作らない）。（v3.1 の「足す」は (i) の前提だった。）
4. **marker は libtest の capture を通さずに stderr へ直接書く**（Sol v2 minor 1）。probe 7：capture の下の `eprintln!` は `abort()` で消える。`std::io::stderr().write_all` は残る。判定は終了コードではなく出力の行で決める（Cargo は abort でも panic でも 101 を返す）。Cargo を通した試験を gate に置く（§6.4）。
5. **二重登録は，登録を作る前に拒否する**（Sol の note。§4.1）。
6. (v3.1) **主張を狭めた所**：§8 の表と Q13。段 M の後に正直になるのは「mutate.py の timeout，build，signal，watchdog」と，**`session_with` と `UhdRig` の constructor を通る生成**の上限まで。FakeDevice の Run を作る所は `session_with` の他に 12 か所あり（§5.4 の一覧：`ezsdr-acceptance/tests/uhd.rs:51, :66, :264`，`ezsdr-radio-uhd/tests/common/measurement.rs:1904` など），M では分類されず，包み（段 2a）の後。Run の途中の上限（KC-21a），cleanup の上限（RS-8a），join も包み（段 2a）の後。包みの無い rig（Kernel の `device_paced.rs`，Python）は対象外。
7. v1 §7 に残っていた「予行は駆動 rig の上で決定的になる」は**取り下げ**（Sol v2 minor 2。§9）。
8. 新しい問い Q15（実機で conclusive な trial が 0 の case）。Q7〜Q14 の見直しは §10。

**(v3.1) v3 から変わったこと**（Opus の再確認 `recheck-v3.md` の指摘。方向は変わらない）

- (v3.1) **M1**：§5.3 の優先順位 5 が読み飛ばすのは，遅く受け取った Action 自身の証拠（その `TIME_ERROR`，drop，拒否）だけ。trial の中の他の fault は FAIL。
- (v3.1) **M2**：Kernel の cleanup の step ごとの上限（RS-8a，5 s，`cleanup_failures[].timed_out = true`）を infra の上限に数える（§5.3 の 1，§6.1，§6.2，§6.4，§7.2 の段 0，§8）。§6.2 の「どれも包みの呼び出しの中で切れる」を直した。
- (v3.1) **m1**：§6.2 の仕組みの書き方（試験から `Stopping` は見えない。理由は `state()` の `CleanedUp` から読める）。KC-21a の呼び出し 3 か所と fixture の対応。
- (v3.1) **m2**：「終わりを期待する」宣言は期待する termination を名指しし，§6.2 の 3 だけを外す。上限の分類は常に行う。
- (v3.1) **m3**：直接 `connect` する 3 か所を書き（数え直すと，`session_with` を通らない生成は `start_spec_run` を含めて 12 か所：§5.4），走査に `connect(`／`start_spec_run(`／`RunHandle` を足した。
- (v3.1) **m4**：R1-tx の閾値は試験側の literal。
- (v3.1) **m5**：優先順位 3 は UHD の Provider に宛てた Action だけ。R1-k の証人は owner の確認待ち（Q17）。
- (v3.1) **m6**：#96 の後の main に合わせた（14 本，§7.1 の件数，Q4 は済み）。
- (v3.1) **m7**：Q16（fake の予行の timing の INCONCLUSIVE：marker つきの失敗か，新しい Run でのやり直しか）。
- (v3.1) §0 の 6 と §8 の言い過ぎ 3 か所を直した。

## 1. 診断

### 1.1 副機構は 3 つ

| 副機構 | 何が起きるか | 該当 | 扱い |
|---|---|---|---|
| **S1 駆動役が時計の外** | 手動の時計は owner のスレッドだけを待つ（M1：`host.rs:8-10`）。試験のスレッドは参加者でない（M6：`host.rs:19`）ので，それが「時刻 T に着いた」と知ってから action が Provider に届くまで，device の時刻は実時間で進む | #69 の 5/6 本，10-27 の 7 本，#76 の `ur_22_a`×2 と `ur_24`，#83 の parity，#87，#96 | 直結の rig：A で閉じる。Run の rig：閉じない（§5） |
| **S2 実時間の FakeDevice** | `ClockMode::Real` では owner の遅れもそのまま device の遅れになる | #60，#68，#69 の `ur_21` の実時間の回，#76 の H4，#83 の測定 group 2 本（INFERRED：issue の文から） | 印をつけて数え，行を `bound` と宣言（§6.5） |
| **S3 ms の実時間の上限が判定** | device を使わないテストが `elapsed() < 200 ms` などで判定する | Kernel `tests/device_paced.rs` に 12 本 | P3 で書き直す（段 3） |

共通点は「判定に入る量を，どの時計にも属さないスレッドが決めている」こと。#87・#96 は probe で VERIFIED。#83 の parity は `UhdRig` が `f487877` から手動の時計なので S1（VERIFIED：`git log -S"ClockMode::Manual"`）。#68・#69・#76 は issue と note 37 の文からの INFERRED。

### 1.2 計画 30 と note 37 が閉じなかった理由

- 計画 30 は参加者を「uhd crate が起こすスレッド」に限り（M1），試験のスレッドの遅れは `hold()`（段階 B）に回して，負荷の表が 0 件だったので作らなかった。表は駆動役だけが遅れる形を作っていなかった（note 37 §1，INFERRED）。
- note 37 は `hold()` を退けて書き手の規則にした。規則は機械で検査されず，#76・#87・#96 と 10-27 の 7 本が後から破った（VERIFIED：failure-mechanisms.md:19, :61）。
- crate の中には止める手段がある（`Frozen`，`released`：`test_support.rs:139-186`）。統合の rig にだけ無い（`spawn` は `pub(crate)`：`host.rs:387`）。直近 3 件（#83・#87・#96）はすべて `UhdRig` で起きた。
- #96 の見立て：「`uhd-tx` が遅れた」は起きない。owner が busy なら時計は止まる（M3：`host.rs:118-128`，M4：`:228-252`）。遅れたのは試験のスレッドである（probe 1）。

### 1.3 probe（各設定 1 回）

| probe | 変更 | 結果 |
|---|---|---|
| 1a | なし | parity の s15 の 10 本とも通過 |
| 1b | `CancellationPair::new` の `uhd.settle` の後に実時間 20 ms | 4 本失敗。gate の失敗と同じ値 |
| 1c | `submit` の `uhd.wait_until` の後に 20 ms | 7 本失敗：#96 の 5 本と，載っていなかった 2 本 |
| 2 | 駆動役を参加者に（`host.rs` に約 10 行，`join` の待ち 3 行，`UhdRig` に 1 欄） | parity 47 本が，注入なし／20 ms×2 か所／200 ms の 4 通りとも通過 |
| 4 | probe 2 から `join` の変更だけ戻す | 6 本失敗（「left detached」）。`join` の変更は必須 |
| 3 | `common::fake()`（Kernel の Run）で同じ登録 | 3/3 が `KC-12a: arm … did not return within 5000 ms`。Run の rig には使えない |
| 5a | `join` の変更だけ | lib 514 本，`fake` 398 本とも通過。非参加者の経路は変わらない |
| 5b／5c | 登録なし。`RunHandle::now()` の戻り／`advance_to` の戻りに 60 ms | `fake` で 10 本／2 本失敗。`uhd.rs` 17 本は通過 |
| 6 | 注入点 3 つ × 遅延 4 つの sweep | §5.2 の表。落ちる集合は注入点で変わる |
| 7 | libtest と Cargo（リポジトリの外の crate，local） | (a) capture の下で `eprintln!` → `abort()`：marker は**出ない**。(b) `io::stderr().write_all` → `abort()`：marker は stderr に出る。どちらも `test result:` の行は無く，Cargo は `(signal: 6, SIGABRT…)` と書いて 101 を返す。(c) 直接書いてから panic：marker は stderr，`test result: FAILED` は stdout，Cargo は 101。(d) filter `inner::passes` は module path に当たる。(e) 通ったテストの出力は `--show-output` のときだけ出る |

busy loop の負荷では #96 は 7 回で 0 件だった。負荷の試験はこの型の検出器として弱い。

## 2. 代替案

| 案 | 型ごと直るもの | 残るもの | 費用・危険 | 判断 |
|---|---|---|---|---|
| **A 駆動役を参加者に** | 直結の rig の S1 全部。テストの文は変えなくてよい | 契約 3（§3）。Run の rig | `host.rs` に登録の口，`join` 1 か所，rig 3 つ。M1 と UR-33 が変わる | **採用（Q1）** |
| B 試験が時刻を step する時計 | A と同じ | A と同じ | 時計のモードが 1 つ増え，M1〜M7 と行が二重になる。M5 を外すと Kernel の非参加者が追いつけない（計画 30 §3） | 退ける |
| C 「観測した条件まで進める」API だけ | なし | テストごとに観測点を選ぶこと。受け取りの時刻で結果が変わる parity は書けない | 小 | Run の rig と実機の原則として残す（P1） |
| D 機械の検査だけ（lint＋負荷） | なし | 全部 | 負荷は #96 を 7 回で 0 件 | 退ける |
| E `Frozen`／`hold()` を外に出す | hold を置いた所だけ | hold を置くこと＝規則のまま | 小 | 退ける（A は既定で，外す方法が無い） |
| F lead を延ばす・流し直す | なし | — | 計画 30・note 37 が退けた | 退ける |
| Run の rig に常時の遅延注入＋走査（Q3 (a)） | なし（検出） | 実時間の遅延は device の時刻の遅れを保証しない（参加者が busy なら時計は止まる）。1 つの遅延は単調でない窓を見逃す。走査は時刻の出どころを数え切れない | 小 | **退けた**（Sol の最初の review の major 2・3）。注入は診断に残す（§5.4） |
| (b′) 包みの呼び出しの外でだけ参加者 | 試験のコードの時間 | 呼び出しの中の Kernel のコードの時間（`ur_25` の形：failure-mechanisms.md:61） | 仕組みが 1 つ増え，「閉じた」と誤読される | 退ける（Q12） |

A が後でできなくすること：登録していない別スレッドから決定的に操作すること。

## 3. 原則と，書き手への契約

- **P1 観測した進み**：「進んだ」と言えるのは Provider か device が残した記録だけ。試験のスレッドが読んだ時刻と，それに足した定数は観測ではない。遅れの証拠は判定を差し控えるだけで，判定を作ることはない（§5.3b）。
- **P2 時計の持ち主**：判定に入る時刻の差は，1 つの時計の上で，その時計の参加者だけが作る。
- **P3 実時間の上限**：使ってよいのは「満了＝失敗」の上限だけ。大きさは秒，通る側の所要の 100 倍以上。ms の上限と「実時間の区間に N 個以上」は判定にしない。
- **P4 実時間の FakeDevice**：使うテストは印で数えられ，それを filter にする mutation の行は `bound` と宣言する（§6.5）。
- **P5 狭い窓**：poll 数回ぶんより狭い窓は `Frozen`／stepped の unit rig。
- **P6 実機**：時計は止められない。予行で規則の境界を固め，実機では P1 だけで判定する。
- **P7 判定の差し控え（改）**：予行で trial を判定しないのは，§5.3b の遅れの証拠があるときだけ。受け取りの後の遅れも証拠に数える（RM-15）。証拠の無い trial は全部そのまま判定する。（旧 P7「受け取りの記録が窓を過ぎたときだけ。受け取りの後は全部 Provider の責任」は取り消し：RM-15 と食い違う。）

契約：

1. 直結の rig（手動の時計）では，device の時刻は rig の進める呼び出し（`wait_until`，`settle`，`wait_for_events`，`finish`）の中でだけ進む。
2. 「T まで進めてから操作する」と書いてよい。操作は **push した時計の時刻**の後の最初の owner の poll（1 ms：`control.rs:250`）で受け取られる。待ちは T の 1 µs 後に返るので，push の時刻は T + 1 µs で，T のすぐ後に poll があればそれが push の時刻に当たり，受け取りはその次の poll になる（例：poll が S + 1 ms と S + 2 ms，T = S + 999 µs なら push は S + 1 ms，受け取りは S + 2 ms。段 1c の Sol の minor）（段 1b・1c の後：rig の，試験が選んだ時刻で終わる待ち（`UhdRig` と `Direct` の `settle`，`wait_until`，`Direct::start`）は 1 µs だけ時計の上で待ち，その時刻に来る参加者を先に走らせる。さらに rig の Action の待ち行列は push の時計の時刻を記録し，時計が**それより後**になるまで渡さない（`common/rig.rs` の `Stamped`）ので，受け取りは push の時刻より後の最初の poll に決まる。`reviews/n51-1b-dispositions.md`）。
3. 駆動役は owner の進みを時計の待ち以外で待ってはいけない。待つと時計が止まり，**毎回**実時間の上限で落ちる。その失敗は marker を持つ（§6）。
4. 駆動 rig が示すのは「受け取りの時刻を決めたときの規則」で，「host が間に合う」ことではない。わざと遅れたいときは `rig.settle(d)` と書く。
5. Run の rig と実機では 1〜2 は成り立たない。窓に依存する判定は書かない（R0 だけ残す）か，予行なら §5.3b の遅れの証拠で囲う（証拠があれば判定せず，単独で流し直す）。

## 4. A の中身

### 4.1 時計ごとの登録

今は thread-local に id を 1 つ持ち（`host.rs:40-43`），id は時計ごとに 0 から振る（`host.rs:267, :402`）。時計 A に登録したスレッドが時計 B で待つと，B の同じ番号の参加者を idle／busy にしてしまう（VERIFIED：コードの読み）。駆動役は rig を 2 つ持てるので起きる。

- 口：`HostClock::drive(&self) -> Driver`。`Driver` は `!Send`，drop で外れる（unwind を含む）。
- 時計の同一性は `Arc::as_ptr(&inner)`。guard が時計の clone を持つので，登録中に番地は再利用されない。
- thread-local は `(時計, id)` の小さな列。`park`（`host.rs:234`），`acquire_lock`（`:431`），`ParticipantGuard`（`:517, :525`）は「この時計での自分の id」を引く。
- **同じ時計に二度**：拒否（panic）。**順序**：`drive()` はまず thread-local の列を引き，この時計があればそこで panic する。`register`（`host.rs:399-402`）はその後なので，拒否された呼び出しは参加者を 1 つも作らない（rollback は要らない）。
- drop は順不同：guard は自分の `(時計, id)` だけを消す。
- 実時間の時計：何もしない guard（`register` が `None`：`host.rs:399-400`）。
- manual と real を 1 つのテストで：real の rig で待つ間，manual の時計は止まる（意図どおり）。
- manual の rig が 2 つ（`parity.rs@fe8:528-537`）：片方で待つ間もう片方は止まる。同時に進めたいテストは今は無い。probe 2 は複数の時計の証拠ではない。
- M1 の文：「uhd crate が起こすスレッド，および `drive()` で登録した rig の駆動役」。M2〜M7 は変えない。

1a のテスト（rows つき）：(1) 駆動役が `std::thread::sleep(200 ms)` しても時刻は動かない，(2) 2 つの時計に登録したスレッドが片方で待っても，もう片方の参加者の状態は変わらない，(3) 同じ時計への二度目は panic し，**その後も参加者の数は 1 のまま**で，最初の guard を drop すると時計は自由に進む，(4) 登録と逆でない順の drop の後，両方の時計が進む，(5) guard を持ったまま panic した後，時計が進む，(6) real では登録が無い，(7) manual と real を持つスレッドが real で待つ間，manual は止まる。

### 4.2 駆動役が時計以外で待つ所

本番の Provider にある時計以外の待ちは 4 つ（VERIFIED：`src/provider/*.rs` の `mod tests` より前，`authority.rs`，`device.rs` の grep）。

| 駆動役の呼び出し | 待ち | A の下で |
|---|---|---|
| `DeviceAuthority::new` | `first_read.recv_timeout(5 s)`（`authority.rs:468`） | 登録をこの後に置くので今と同じ |
| `prepare`／`arm`／`start`／`cleanup` | 無い。`apply_delay` は `host.sleep`（`device.rs:1394`） | そのまま（parity の FakeConfig は probe 2 で VERIFIED，`Direct` と fault つきは INFERRED） |
| `wait_until`（root）／`settle`／`wait_for_events` | `host.wait`／`host.sleep`。`wait_for_events` の 10 s は実時間の上限（`parity.rs@fe8:444-455`） | そのまま。上限は marker に |
| `wait_until`（`host.monotonic`） | `std::thread::sleep`（`authority.rs:258-266`） | rig は root しか渡さない（`parity.rs@fe8:408-410`） |
| `stop` → `Provider::join` | `std::thread::sleep(1 ms)` の loop（`provider/mod.rs:396-398`） | **`host.wait(None, 実時間 1 ms, seen)` に**（Q2，probe 4）。`JOIN_BOUND` は実時間のまま |
| rig の Drop（unwind を含む） | `DeviceAuthority::drop` の `std::thread::sleep` の loop，上限 1 s（`authority.rs:498-510`） | guard を rig の最初の field にする。Authority が drop する時点で駆動役はもう参加者でない |
| テスト側の channel の `recv`，`bounded`（`fake.rs:1790-1803`），`thread::scope` の join | 時計以外 | 段階 0 の W 表で 1 本ずつ：時計で待つ recv に替えるか，実時間の時計のまま |

### 4.3 模擬の遅延と fidelity

- `apply_delay`（`device.rs:1394`）と `SlowSend`（`:1785-1788`）は既に `host.sleep` で，A はこれを消さない（VERIFIED：コードの読み）。規則：模擬の遅延は時計の待ちで書く。
- `TxBlocks` は実時間の時計のまま（`device.rs:1778-1783`）。使うのは `fake_real` だけ（VERIFIED：`grep -rn TxBlocks crates`）。Manual を強制する rig の constructor に「`TxBlocks` を受け取らない」assert を足す。
- A の下では，駆動役の書いた `std::thread::sleep` も device の時刻に現れない。実機ではあり得ない（契約 4）。駆動 rig のテストでは `thread::sleep` を走査で禁じる。

### 4.4 `drop(frozen)` の 17 か所（VERIFIED：各行の前後。1d で数え直した：§4.4 の最初の表は 15 か所で，`drop(rig._frozen)` の G と `tx/tests/e2b.rs:88` の W2 を落としていた。場所は 1d の後の行）

| 型 | 場所 | release の後 | 扱い |
|---|---|---|---|
| N（4） | `tx/tests/o1.rs:755`，`tx.rs:4396, 4463`，`rx.rs:1000` | assert だけ | 変えない |
| G（4） | `tx.rs:6555, 6570, 6593, 7003`（7003 は `drop(rig._frozen)`） | 試験のスレッドが owner のコードを自分で走らせる | guard：`test_support::handed_over(frozen, &clock)` = `let driver = clock.drive(); drop(frozen); driver`（試験のスレッドを先に参加者にしてから時計を渡す）。1d で入れた |
| W1（5） | `tx.rs:5952, 6059, 6092`，`rx.rs:901, 946` | owner は参加者。試験のスレッドは channel か実時間の poll で待つだけ | 変えない（guard を足すと hang） |
| W2（3） | `tx.rs:6542`，`tx/tests/e1.rs:88`（`drop(_frozen)`），`tx/tests/e2b.rs:88` | 150 ms 眠る参加者を起こし，`shutdown`，`join` | 眠る helper と join を消して `handed_over`（`join` は時計の外の待ちなので，joined な thread が残る限り guard は hang する）。1d で書き直し，9 本が通る（VERIFIED） |
| H（1） | `provider/test_support.rs:182` | 仕事を参加者に渡す helper そのもの | 変えない |

## 5. Run を通す rig（Q3 (c)）

### 5.1 なぜ A で閉じないか

Kernel は試験のスレッドの上で時計以外の待ちをする：KC-21a（`coordinator/paced.rs:177-205`），KC-12a（`paced.rs:210-223`，probe 3）。Provider が Action を受け取るのは `uhd-control` が `host.sleep(POLL)` から戻った後（`provider/control.rs:234-250`）なので，その待ちの間は時計が進まなければならない。Action が Provider に届くまで，試験のスレッドは必ず Kernel のコードを非参加者として走る。閉じるには Kernel の待ちを時計の待ちにする口が要り，AGENTS §3 と計画 30 §3 に反する。

### 5.2 分類と調査

| 級 | 意味 | 扱い |
|---|---|---|
| R0 配線 | 判定が「Action がいつ届いたか」に依存しない | Run の rig に残す |
| R1 lead | 「target − lead」より前に届くことが要る | on time が主題でなければ assert を外す。主題なら直結の rig へ |
| R2 phase | device の 2 つの時刻の間に届くことが要る | 窓の場面を直結の rig へ。Kernel を通ってしか見えない事実は，窓を使わない形で Run に残す |

probe 6（`RunHandle` の 3 か所の戻りに実時間の遅延。`fake` binary 398 本の失敗数）：

| 注入点 | 5 ms | 20 ms | 60 ms | 200 ms |
|---|---|---|---|---|
| `now()` | 2 | 7 | 10 | 10 |
| `advance_to` | 0 | 2 | 2 | 2 |
| `submit` | — | 4 | 5 | — |

- `submit` の注入だけが落とす 2 本がある（`common::measurement::tests::rx_start_lead_group_runs_on_the_fake`，`uhd.rs` の `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end`）。合わせて 12 本。この 2 本は読んでいない。
- `protocol.rs` の 74 本はどの設定でも 0 本（Kernel の中の注入なので，返事の `now` を読んでから動く形は見ていない）。
- 遅延の大きい集合は小さい集合を含んだ。単調でない窓は INFERRED のまま。

`now()` の注入で落ちた 10 本：

| テスト | 級 | 根拠 | 扱い |
|---|---|---|---|
| `n47s5_front_…`，`n47s5_long_close_…`，`n47s5_repeat_switch_…` | R1 | 予約は「今＋15／40 ms」，capture は「今＋1〜5 ms」（`tests/common/n47s5.rs:174-196, 218-231, 264-281`） | 移せない。§5.3 |
| `n47s5_untimed_…` | R1 と R2 | capture「今＋1 ms」（`n47s5.rs:348, 375`），「A の開始＋5 ms の後，A が鳴っている間」（`:381-384`） | 移せない。§5.3 |
| `n47s5_held_tail_and_stop_…`，`n47s5_one_sample_after_a_burst_…` | R2 | `while now_root(&run) < j_root + lag`（`n47s5.rs:460-467`），Stop の直後の予約（`:523-538`），`late_booking`（`:408-412`） | 移せない。§5.3 |
| `ur_21_s18_…` | R1（10 ms） | `fake.rs:2037-2047`。主題は予約の規則 | 直結へ |
| `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` | R2 | `fake.rs:1885-1925` | 窓の半分を直結へ。「拒否が burst の Action id を持つ」は Run に残す |
| `ur_23_a_burst_after_a_stop_…`，`ur_29_a_loss_found_by_…` | R1（10／20 ms） | `fake.rs:2223-2237`，`:3756-3770` | 段階 0 で決める |

1 本ずつの R 表は**まだ無い**。段階 0 の成果物で，範囲は FakeDevice と Run を一緒に作る test file 全部：`ezsdr-radio-uhd/tests/fake.rs` と `tests/common/**`（`fake` binary の 398 本のうち `fake.rs` 本体 256 本は helper の名前で Run を通すもの 65 本＋予行 21 本，`tests/common/` の 142 本は未調査），`ezsdr-acceptance/tests/uhd.rs`，`ezsdr-server/tests/protocol.rs`。probe 6 で落ちなかったことは R0 の証明ではない。

**移せないもの**：実機テストと列を共有する予行。`tests/common/{n47s5,n47,d9,measurement,iii}.rs` の trial は `hardware.rs` の `#[ignore]` のテストと `fake.rs` の予行の両方から呼ばれる（VERIFIED：`n47s5.rs:1-3`）。AGENTS §9 の予行の規則は「同じ境界を fake と実機の両方で」なので，直結の rig に移すと予行でなくなる。

### 5.3 予行：遅れの証拠と，判定の差し控え（§5.3b が置き換えた）

**今の状態（VERIFIED）**

- Provider は受け取りの時刻を読んでいる（`let receipt = self.core.now()`：`control.rs:484`，値の更新は `:283`）が，記録するのは switch の行の `selected_at` だけ（`control.rs:591`）。
- `TIME_ERROR` は受け取りの所（`control.rs:510-625`）と dispatch の所（`tx.rs:1054-1126`）の両方が同じ outcome（drop，plan_violation，refused，send_asap）で出し，payload に出どころは無い（`control.rs:463-469`，`tx.rs:1048`）。**今の記録では「受け取りで遅れた」と「受け取った後で遅れた」を分けられない。**
- `late_arrival_verdict`（`iii.rs`）は late の drop を見て「late at receipt or at dispatch … host latency」と INCONCLUSIVE を返す。実機の guard と N7 の判定として今のまま残す（作り直さない：§5.3b）。Session の Stop の N2 自身の late の drop も同じ判定（`late_arrival_verdict_of("N2", …)`，名前だけが違う）を通し，N1 の行は N2 の late の drop の数だけ `TIME_ERROR` を引く（fix 1）。N7 の 2 case では，この関数に入るのは C 以外に遅れの証拠が無い run だけ（`Rehearsal::trial` が先に差し控える：段 2c C3）。
- fake の予行の assert は 10 か所が「`Fail` でなければよい」，2 か所が「`Pass`」（`fake.rs` の grep）。INCONCLUSIVE は fake でも通っている。

**§5.3b への 1 段落**：予行（fake）の trial の判定は，§5.3b の遅れの証拠（`TIME_ERROR` の cause が late，または Kernel の log の時刻が窓を過ぎている）があるかどうかだけで決まる。証拠があれば判定せず，印 `EZSDR-TIMING:` を書いて単独で流し直す。無ければ全部そのまま判定する。級ごとの窓と証人（R1-tx，R2，R1-k），Provider の `receipts`，`receipt_class` の読み飛ばし，優先順位 3〜6，`late_arrival_verdict` の作り直しは作らない（owner 2026-11-13）。

**優先順位**（残るのは 1 と 2。上から順に，最初に当たったもの。2 は証拠より先に読む：admit 違反と遅れの証拠が同じ trial にあれば admit 違反が勝つ）

| # | 条件 | 結果 |
|---|---|---|
| 1 | Run が実時間の上限に当たった（§6.2 の 2：`KC-12a:`／`KC-21a:` の理由，`cleanup_failures` の `timed_out: true`（RS-8a），`did not join within`） | fake：marker つきの失敗。実機：今のまま失敗 |
| 2 | Kernel が admit すべき Action を admit しなかった（trial の全 Action。`submit` が `Err` を返したものを含む） | **FAIL**（印なし） |

**2c の純関数の test**：`late_evidence` の 2 条（境界つき。`late_at_device` は証拠にしない），`Rehearsal`（判定を飛ばす・印は最後に・plain な失敗には印なし・admit 違反は証拠より先に plain な失敗），`kernel_refused`，予行 9 本の trial の数と `finish()` を見る `governance.rs` の規則（fixture つき），literal と本番の定数の一致，Stop の fault 先行，long close の FAIL，N7（C だけが遅い run は判定される・他の Action の遅れは証拠のまま残る：`iii::without_expected_late`）。

- **fake の予行での出方**：証拠のある trial は判定せず，`EZSDR-TIMING:`（`common::timing`）の印を最後に 1 回書いて失敗させる。Q16 b″ のとおり，`mutate.py`（`RERUNS`）と `gate.py`（`excuse_timing`）が単独で 2 回まで流し直す。3 回とも印なら gate は赤，mutation は `ERROR`。わざと遅く受け取らせる case（N7：C を e − 0.5 ms に出す）も `Rehearsal::trial` に入る。`Lateness` から C 自身の late の drop（`late_arrivals` の数だけの `(late, drop)`）と C の deadline を除くので，C だけが遅い run は判定され，共有の判定が「INCONCLUSIVE（host latency）であること」を assert する。C 以外（A や capture）にも遅れの証拠があれば判定せず印を出す（単独で流し直す）。
- **timing 以外の INCONCLUSIVE**（capture に onset が見えない，など測定の不成立）はこの設計の対象外。
- **実機**：遅れは変えない。今の guard のまま `Verdict::Inconclusive`（`tone.rs:19-25`：INCONCLUSIVE はテストを通す）。変わるのは，遅れの報告が無いのに長い close の形が欠けた trial が FAIL になること（M4）だけ。Q15（実機で conclusive な trial が 0 の case を FAIL）は作らない：前提（(i) が取り消され，実機に遅れの新しい INCONCLUSIVE の出どころが無い）が無くなった（orchestrator 決定 3）。
- **失うもの**：§5.3b の「失うもの（正直に）」。
- 予行は負荷で落ち得る（lead 15〜40 ms は実機と同じ）。**閉じてはいない。囲う**。

### 5.3a 拒否の扱いの訂正（v3.1-orch，orchestrator 2026-11-12，Opus の再確認 recheck-v3.1.md の N1）

上の優先順位 5 の「遅れた Action 自身の拒否は読み飛ばす」を次に置き換える．
- **受け取りの時刻を読む前に決まる拒否**（`control.rs` の :419-463，:485-487 の類：waveform，形，重なり等）は遅れと関係がないので，**常に FAIL**（今の `iii.rs:1233` の向きを保つ）．
- **受け取りの後に決まる拒否**のうち，級ごとに許した理由だけを優先順位 5 で読み飛ばす．許可表（級 × 理由）は段階 2c の R 表で決め，Sol のテスト計画レビューを受ける．表に無い理由は FAIL．
- （`receipts` は作らない：§5.3b。この項は取り消し）
- 今の `stop_booking_verdict`（`iii.rs`）は，Stop の受け取りの遅れで後の予約が拒否されたとき INCONCLUSIVE にしている．許可表は §5.3b が捨てた．2c は N1 の行の overlap の拒否（INCONCLUSIVE，fault が先）を変えず，N2 の行を足した．N1 の行は，N2 自身の late の drop の数だけ run の `TIME_ERROR` から引く（fix 1：N2 だけが host の遅れで late の drop になっても N1 の行を FAIL にしない）．

その他の訂正（N2〜N5）：
- N2：RS-8a の検査を逃れるのは「Run の終わりの後に包みを一度も呼ばないテスト」だけで，Manifest を読まない．残り得るのは包みを離した後に device の記録を読む形だけ．2a の対策（走査か `#[must_use]`）は段階 0 でその形があるかを見てから決める．
- N3：`common/mod.rs:460` の helper `spec_run`（`fake.rs` から 39 行）は `session_with` と同じ file にあるので，段階 M に入れる．
- N4：Q16 の推奨の文から「予行の力も落とさない」を消す（(b) が失うものと食い違う）．
- N5：`LogEntry.time` は片側の証人なので，submit の中で遅れた capture は判定され，偽の kill になり得る．§8 の予行の行にそう書く．
- `measurements.rs` と `x3xx.rs` は Run を作らないので，§5.4 の「読んでいない」から外す．

### 5.3b 遅れの証拠がある予行の trial は判定せず，単独で流し直す（owner 2026-11-13．§5.3 の優先順位 3〜6 と §5.3a を置き換える）

**決定**：予行（FakeDevice で実機と共有する trial）で**時刻の遅れの証拠**が出た trial は，その回は判定しない．timing の印（`infra::timing`，`EZSDR-TIMING:`）を書き，そのテストを**単独で**流し直す（Q16 b″ の仕組み：`mutate.py` の `RERUNS`，`gate.py` の `excuse_timing`．どちらもある）．流し直しで証拠が出なければ，その回の結果で判定する（FAIL は FAIL）．**3 回とも印が出たら**，gate ではその段を赤（「timing-marked」は調べるべき FAIL として扱う）にし，mutation の行は `ERROR` にする（owner 2026-11-13，M1．流し直しの回だと test に知らせる仕組みは置かない：並列の shard で誤った kill が戻るため）．遅れを生む変異は，時計を止めた rig（direct rig）の test で殺す．実機の遅れの扱いは変えない（遅れた trial は今どおり INCONCLUSIVE で数えない：遅れは実機では INCONCLUSIVE のまま）．変わるのは，遅れの報告が無いのに長い close の形が欠けた trial が FAIL になることだけ（Sol の M4）．

**理由**：P7（owner (i)：受け取りの後は全部 Provider の責任）は RM-15（design/07：受け取りの後でも host の遅れや回復で dispatch が遅れた予約は，そこで late policy で判定する）と食い違う．Fake で T − 2.035 ms に受け取った予約が，uhd-tx の手番が 0.2 ms 遅れて drop された（2c の計画の F1）．これは仕様が許す振る舞いである．P7 の上に「遅れた Action 自身の証拠だけを読み飛ばす」規則を組むと，級ごとの窓，Action の結末（drop・送信・拒否）ごとの読み飛ばし，拒否の許可表が要り，Sol の計画レビューと Opus の再確認で major が計 9 件出た（材料は `reviews/n51-2c-*`）．仕組みが規則の数で増え続けるので，仕組みを減らす（AGENTS §6）．

**遅れの証拠**（どれか 1 つ．判定を差し控えるだけで，判定を作ることはない：P1 もそう改める．試験が submit の前に読んだ時刻は，Kernel の stamp がその後に取られるので 2 つ目に含まれ，別の証拠にしない〈orchestrator 2026-11-13〉）：
- Provider が報告した `TIME_ERROR` で **cause が late** のもの（受け取りでも dispatch でも．どちらも host の遅れとして仕様が許す）．late 以外の `TIME_ERROR`（design/07 の他の用途）と `late_at_device`（Provider は自分の時計で間に合って渡した：手渡しの regression を指す）は証拠にしない．読むのは `iii::time_error_causes` の 1 か所．
- Kernel の action log の時刻（`LogEntry.time`，Q17 の片側の証人）が，試験が狙った窓を過ぎている Action．窓の端は試験側の literal（本番の定数から計算しない．literal と定数が等しいことは別の test で見る）．
- （計画値の注意）`stop_root` は Kernel の stamp，`capture_at`／`tx_at` は前に読んだ `now()` から作った計画値で，実際の時刻ではない．

**どこで読むか**：Fake 側の helper 1 つ（上の 3 つを読む）を，共有の判定（`late_arrival_verdict` など）と Fake だけの assert（`fake.rs` の `late_by.is_none()` など）の**前**に呼ぶ．trial の中の `assert!(admitted(..))` も分類の後に回す（遅れた capture も admit される，INFERRED）．Fake の規則は実機の guard より広い（実機は受け取りの遅れと dispatch の遅れを分けられない）．trial を複数持つ test は，印の出た trial を飛ばして残りを流し，最後に失敗させる．対象の test は 2c の計画に列挙する．

**baseline**：mutation の baseline も同じ `judge` を通す（今は exit 0 しか受けず BASELINE FAILED になる．2c が `timing()` を初めて出すので，予行を filter に持つ行 N47S5-177/192/193/194，N47B-19/36 などが巻き込まれる）．

**予行の assert を切り替える前に**：単独の 20 回で印 0 を確かめる．単独でも試験の時計は実時間で進む（`src/host.rs`）ので，余裕の小さい case は単独でも印が出る．held-tail の J(e) 後の case（T_C − 受け取り 1.54〜2.45 ms，12 回中 5 回が 2 ms 未満）は F2（共有 trial の C = 1 000，実機側の点検つき）を，`stop_next_trial`（Stop の受け取りが Kernel の stamp の約 0.9 ms 後．どの証拠にも当たらず毎回拒否）は F3 を 2c に含めて直す．

**残すもの**：§5.3 の優先順位 1（実時間の上限：marker つきの失敗）と 2（Kernel が admit すべき Action を admit しなかった：FAIL．証拠より先に読む：同じ trial に admit 違反と遅れの証拠があれば admit 違反が勝つ）．遅れの証拠の無い trial は全部そのまま判定する（late の drop も underflow も，他の Action の fault も FAIL）．わざと遅らせる case（N7）も述語を通す：C 自身の期待された late の drop と deadline を除いた `Lateness` で `Rehearsal::trial` に入れ，C だけが遅い run は今の共有の判定（`late_arrival_verdict`）で判定する．他の Action の遅れの証拠は除かないので，その run は判定せず印を出す（N7 を述語の外に置くと，注入した遅れが A も遅らせたとき plain に落ちた：段 2c C3）．

**捨てるもの**：§5.3 の優先順位 3〜6，§5.3a の許可表（§5.3a の N2〜N5 は残る），`receipt_class` の読み飛ばし．`receipts`（Q10）は判定に使わない．診断に要るかは別に決める（今は作らない）．

**失うもの（正直に）**：
- 遅れた回の中で別の Action に故障があっても，その回では捕まえない．単独の流し直しで再現すれば捕まる．
- 負荷の下でだけ遅れを生む Provider の regression は，単独では再現しないので捕まらない．`gate.json` の timing の流し直しの数が増えることでしか見えない（gate の記録で見る）．
- Action を遅らせる変異は，予行では `killed` にならず `ERROR` になる．今 guard（`iii.rs` の実機の guard）経由で死んでいる行は，direct rig の test を filter に足す（M-R の範囲が広がる）．
- `LogEntry.time` は片側の証人なので，submit の中で遅れた capture は判定される（残る誤判定：N5）．
- 2b の `ea_16` の「窓ちょうど」の義務は，置き場所を 2c で決め直す．

**検査**：遅れの証拠を決める純関数（trial の記録 → 証拠の有無と理由）と，印を書く経路の test．Sol の 2 つの反例（受け取りの後の drop，別の Action の underflow）は「遅れの証拠があれば判定しない／無ければ FAIL」で，それぞれの結果を test に書く．N7 は，C だけが遅い run は判定され（述語が除くのは C の drop と deadline だけ），A も遅い run は証拠のまま残ることを純関数の test に書く（`iii::without_expected_late`）．

**§5.3b に合わせて直した他の本文**（2c で直した。残る古い文は履歴で，上の取り消しが優先する）：冒頭と §0 の (i) の行，項目 2〜3；§3 の P7 と契約の項目 5；§5.3 の表，優先順位 3〜6，tests (a)〜(f)，marker 名 `EZSDR-TIMING`；§7.2 の 2c の行；§8 の 4 行；§9 の 3 行；§11 の receipts の項目；付録の Q10／(i) の取り消し（2026-11-13 に記録），Q14 の範囲，Q15 の前提と F3，Q17 の使われない節．

### 5.4 rig の境界と遅延注入

- Run の rig は raw `RunHandle` を渡さない包みにする（`tests/common/run.rs` の `Run`。`Deref` も `into_inner` も無い）。**作った形（段 2a）**：`Run::connect`／`Run::start_spec`（生成。`SpecError` はそのまま返す），`submit`，`advance_to`，`wait_for`，`run_until_end`，`finish(self)`，読むだけの `state`，`now`，`start_instant`，`sample_clocks`，`effective`，`events`，宣言の `expecting`／`join_is_subject`／`rx_join_issue_67`。試験が今使っている method だけで，使われていない `id`，`kind`，`run_child`，`disconnect`，`check_lease` は渡さない。`run.rs` は `#[path]` で acceptance crate の `uhd.rs` も取り込む（`infra.rs`，`watchdog.rs`，`rig.rs` と同じ形。新しい test-support crate も，規則の二つ目の写しも作らない）。§6.2 の分類と診断の注入点は，試験に制御を返す所のここ 1 か所。
- テストがまだ届くもの：包みの method，Manifest，device の記録（`calls()`，`ProbeDevice` の send）。届かなくするもの：`device.time_now()` と `.host()`（走査，fn 単位の許可表。今の例：`tests/common/iii.rs:251-252`）。
- 実機と共有する trial（5 module）も包みを受け取る（Q11）。包みは転送と §6.2 の検査だけで，実機では注入は off。`hardware.rs` の変更は型の付け替えで，compile は gate（`uhd` feature の build）で，挙動は次の bench で確かめる。
- `protocol.rs` は `Server` が境界。**作った形（段 2a）**：`server.handle(` の **82 か所**（1 か所ではない：`let handled = server.handle` が 7，`Server::new(..).handle` が 2，残りは `ok(…)`／`err(…)` の中）を test file の 1 つの fn `handle(&mut Server, Request, Vec<u8>)` に通し，そこを注入点にする（返事が戻った後に `rig_delay("server")`）。`serve`／`serve_bytes`／`serve_watched`／子 process の入口は通さない（FakeDevice を使う 18 本のうち `ea_15` だけが `serve_watched`。`handle` 以外の入口は返事から Run の終わり方を読めないので，分類は §8 のとおり対象外のまま）。
- 走査は構文で決まる事実だけ：テストの中の `thread::sleep`，`Instant::now`，`elapsed()`，`recv_timeout(`，`.time_now(`，`.host()` は helper の中だけ。directory を再帰し，fn 単位の許可表。**作った形（段 2a。`ezsdr-acceptance/tests/governance.rs` の Rust のテスト，`scan(path, source)`）**：comment，文字列，char literal を空にしてから token に切るので，path や `(` の前の空白や改行に左右されない（`connect (`，`coordinator::` の後の改行）。fn は brace の深さで追い，`#[test]` の付いた fn が試験で，試験の中に入れ子にした helper は helper。禁じるもの：(1) 包みの module（`common/run.rs`）の外での `RunHandle`（`RunHandleError` は別の token）と raw の `connect(`／`start_spec_run(`（`Run::` と `fn` の後は許す。呼び出しだけでなく，`use` と関数値としての参照 `let create = coordinator::connect;` も同じ token で止まる）。範囲は `ezsdr-radio-uhd/tests/` の全 file（`hardware.rs` と `infra_fixtures/` を含む）と，`ezsdr-acceptance/tests/` から Run を作らない file（`v58`，`v61`，`reactive`，`parity`，`measurements`，`x3xx`，`o1_oracle`，`repeat_switch`，`governance`，`support/`，`oracle/`）を除いたもの。(2) `Expect::Any` は名指しの 3 fn（`perf_rx_rate_trial`，`b8_lead_trial`，`run_stage`）の中だけ。(3) 試験の fn の中の時刻の出どころ（`fake.rs`，`common/**`，`uhd.rs`。`hardware.rs` は device の時刻を自分で読むので対象外）は fn 単位の許可表 24 行（理由つき）。許可表の行が使われていないと fail する。(4) `protocol.rs` の `.handle(` は `fn handle` の中だけ。走査は「新しい素通り」を止めるもので，helper を経由した時刻の出どころは見えない。
- (v3.1) **生成の抜け**（m3）：FakeDevice の Run を `session_with` の外で作る所が 12 ある（VERIFIED：`grep -rn 'connect(\|start_spec_run(' crates/*/tests` を切らずに読み，uhd の FakeDevice を使う file — `ezsdr-radio-uhd/tests/**` と `ezsdr-acceptance/tests/uhd.rs` — の hit だけを残した。`connect()` という名前の別の fn と `disconnect(` は除いた）。
  - (v3.1) 再確認が挙げた，直接 `coordinator::connect` する 3 つ：`ezsdr-acceptance/tests/uhd.rs:66`（`uhd_session_asking`），`:264`，`ezsdr-radio-uhd/tests/common/measurement.rs:1904`（`candidate_session_asking`。`connect` の `Err` を文字列にして返す）。
  - (v3.1) Kernel の `start_spec_run` を直接呼ぶ 9 つ：`ezsdr-acceptance/tests/uhd.rs:51`（`uhd_spec_run`），`ezsdr-radio-uhd/tests/common/mod.rs:460`（helper `spec_run`），`:785`（`rehearse_overflow`），`common/measurement.rs:2628`，`common/measurement/multichannel.rs:791`，`fake.rs:221, :535, :3786, :6749`。
  - (v3.1) 段 2a でこの 12 を包みの生成に替える（acceptance crate の側には `run.rs` を `#[path]` で取り込む）。それまでは §6.2 の検査を通らない（§8）。段 M が分類するのは `session_with`（`common/mod.rs:494`）と `UhdRig` だけである。**段 2a で替えた**：12 か所と `session_on`／`start_spec` は `Run::connect`／`Run::start_spec` を通り，raw の生成は `run.rs` の中にしか無い（走査が止める）。12 か所の今の場所：`uhd.rs:56, :71, :269`，`common/mod.rs:480`（`spec_run`），`:874`（`rehearse_overflow`），`measurement.rs:1904, :2637`，`multichannel.rs:792`，`fake.rs:536, :3833, :6815`。残る 1 つ（`fake.rs:221`，`ur_06_the_provider_refuses_another_root`）は `spec_run_ended`（`common/mod.rs:485`）を通る。
  - (v3.1) 読んでいないもの：FakeDevice か `ezsdr_radio_uhd` の名前が出る他の test file（`ezsdr-acceptance/tests/measurements.rs`，`x3xx.rs`，`ezsdr-server/tests/protocol.rs`）が Run をどこで作るか（上の grep には当たらない。`protocol.rs` は `Server` 越し）。段階 0 で見る。
- (v3.1) **走査に足すもの**：`connect(`，`start_spec_run(`，型の名前 `RunHandle` は，uhd の rig を使う test file（`ezsdr-radio-uhd/tests/**`，`ezsdr-acceptance/tests/uhd.rs`）では包みの module の中だけ。新しいテストが包みを素通りすると走査で落ちる。包みの無い rig の file（Kernel の tests，`v58.rs`，`reactive.rs`，`support/mod.rs` など）は file 単位で「対象外」と書いた許可表に載せる（§8 の「包みの無い rig」と同じ集合）。
- **遅延注入は診断**：(1) 段階 0 の R 表の調査，(2) 段の終わりに 1 回，注入点×{5, 20, 60, 200 ms} で流し，「注入なしで通り，注入で落ちる」テストを報告する。gate の判定にも mutation にも使わない。限界：参加者が busy なら時計は止まるので，実時間の遅延は device の時刻の遅れを保証しない。grid にしても証明にはならない。**作った形（段 2a）**：`common/infra.rs` の `rig_delay(point)`（`protocol.rs` も `watchdog::infra` から使う 1 つの写し）。環境変数 `EZSDR_RIG_DELAY_MS`（無いか空なら何もしない。数でなければ panic：打ち間違いを「遅延なし」と読ませない）と `EZSDR_RIG_DELAY_AT`（`now`，`advance`，`submit`，`server`，comma 区切り，`all`）を 1 回だけ読み，呼び出しが戻った**後**に呼んだスレッドが眠る：`Run::now`／`start_instant`（`now`），`submit`（`submit`），`advance_to`／`wait_for`／`run_until_end`（`advance`），`protocol.rs` の `handle`（`server`）。包みの `check` の後なので，`check` の panic は遅れない。`finish`，`Drop`，`state`，`events` に点は無い。runtime の読みで `cfg` ではない（acceptance crate に `uhd` feature が無い）。gate にも実機にも設定しない。

## 6. mutation の正直さと，上限の分類（#91 と一緒に）

### 6.1 今

`mutate.py:146` は 0 でない終了を全部 `killed`，`:148` は timeout を `killed (timeout)` にし，`gate.py:555` の self-test がそれを通過として固定している。実時間の上限は A の後も残る：join 1 s（`provider/mod.rs:395-404`），event の待ち 10 s（`parity.rs@fe8:444-454`），watchdog 120 s（`tests/common/watchdog.rs:27`），KC-12a／KC-21a 5 s（`paced.rs:24-26`，env で変えられない），`TEST_TIMEOUT` 180 s。(v3.1) **RS-8a**：Kernel の cleanup は step ごとに thread を起こし，`recv_timeout` で待つ。Provider が宣言しなければ 5 000 ms（`ezsdr-kernel/src/run.rs:545`，`:584`，`:594-604`）。過ぎた step は捨てられ，Run の `termination.reason` は変わらず（Stopped／Completed のまま），Manifest の `termination.cleanup_failures` に `timed_out: true`，`"RS-8a: abandoned after … ms"` と残るだけである（`run.rs:588`，`:520-523`）。v3 の一覧に無かった（再確認 M2）。

### 6.2 上限を，制御を返す前に分類する（Sol v2 major 1）

Sol の列：baseline は通る → mutant が負荷の下で走る → prepare が KC-12a を過ぎる → `connect(...).unwrap()` か Running の assert が panic（`tests/common/mod.rs:494-495`）→ marker なしの `FAILED` → killed。途中の assert（`fake.rs:1899-1916`）でも同じ。

- (v3.1) **Run の終わり方は `finish()` の前でも読める**（VERIFIED。m1 で書き直した）：上限は `ending::request(Termination::Failed { stage, reason })` を呼ぶ（`pipeline.rs:582, 629, 884-886`，`paced.rs:191-198`）。`request` は `shared.end` を置くだけで，state は変えない（`ending.rs:210-226`）。`Stopping` から `CleanedUp` へは，公開の呼び出し（`connect`，`submit`，advance）が戻る前に `settle()` が進める（再確認が VERIFIED：`ending.rs:15-47`，`mod.rs:160, 177`，`pipeline.rs:1048`，`stepping.rs:408, 423, 458`）。だから：
  - (v3.1) 試験のスレッドから `Stopping` は見えない。包みが method の戻りで見る `state()` は Running か `CleanedUp { termination }`（`run.rs:113-117`）である。
  - (v3.1) `connect` は KC-12a で切れても `Ok` を返す。Sol の列で panic するのは `unwrap()` ではなく Running の assert（`tests/common/mod.rs:494-495`）。
  - (v3.1) 理由（`Termination`）は `state()` から読める。`finish()` を呼ばなくてよい。`ending::finish` は `CleanedUp` の後は何もしない（`ending.rs:197-202`）ので，呼んでも害は無く，必ず返る。
  - (v3.1) **RS-8a だけは `state()` から読めない**：`CleanedUp` が持つのは `Termination` だけで，`cleanup_failures` は Manifest の `TerminationSection` の欄である（`manifest.rs:208-221`）。RS-8a を調べるには Manifest が要る。
  - (v3.1) Kernel には何も足さない。
- **包みの検査**（生成と，全部の method の戻りで）：
  1. (v3.1) `state()` が `CleanedUp` なら，**誰が終わりを頼んだかに依らず**，包みが Manifest を取り（自分で `finish()` を呼ぶ。通常の経路で，unwind の中ではない），`termination.reason`，`termination.also`（Policy の後に切れた KC-21a などは Kernel が `also` に残す：`ending.rs` の `request`），`termination.cleanup_failures` を読む。宣言は `also` の上限を免除しない。テスト自身の `finish()`，Stop を運ぶ `submit` の戻りも同じ検査を通る。「テストが終わりを頼んだら検査しない」は採らない：Stop を submit したその呼び出しの中で KC-21a が切れると，`wait_finished`（`pipeline.rs:1037`）が Stop の request（`:1043`）より先に termination を決める。RS-8a の列（下）ではテスト自身が `finish()` を呼んでいる。
  2. (v3.1) **上限**（常に分類する）：理由が `KC-12a:`／`KC-21a:` で始まる，**または `cleanup_failures` に `timed_out: true` の行がある（RS-8a。構造化された欄なので文字列は見ない。Kernel 自身が書く KC-44 の行は `timed_out: false` なので当たらない：`state.rs:262-264`）**，または Provider の記録に `did not join within`（`provider/mod.rs:403`）がある → marker を書いて（§6.3）**そこで 1 回 panic**。テストの後ろの assert は走らない。
  3. (v3.1) それ以外の終わりで，テストが宣言した「期待する termination」に合わないもの → marker なしで panic（「Run が終わった：理由」）。mutant が作った本物の失敗で，kill の証拠になる。テスト自身の `finish()` と Stop による終わり（`Stopped { cause: Client }`）と，Spec の Run の `Completed` は宣言なしで通る。
  - **規則と保証（段 2a の後）**：この検査は，Run を**終わらせ得る**呼び出しが戻る所で走る：`connect`，`start_spec`（生成），`submit`，`advance_to`，`wait_for`，`run_until_end`，`finish`，そして `Drop`。Run が `CleanedUp` に入るのは `settle()` を呼ぶ呼び出しの中だけ（`ending.rs:15-47`；`settle()` の呼び手：`connect` `mod.rs:160, :177`，`submit`／`run_child` `pipeline.rs:1048, :1094`，advance／`wait_for`／`run_until_end` `stepping.rs:408, :423, :458`）なので，**読むだけの method（`state`，`now`，`events`，…）の戻りは何も検査しない**：それらは検査済みの呼び出しが判定していない新しい終わりを見せ得ない。Manifest は `finish` が handle を消費しないと得られないので，read-only の戻りで検査を足しても何も増えない。背景で切れた上限（Policy の終わりが `stop_early` をデータ thread で走らせ（`paced.rs:133-148`），Provider の join が timeout する，など）は，**次の検査される呼び出し，`finish`，または `Drop`** で見える。保証は「Run を終わらせ得る呼び出しの戻りと `finish` と `Drop` で，上限は marker つきの panic（unwind の中は marker だけ），期待しない終わりは panic になる。read-only の戻りは検査しない」で，それ以上は言わない。
  - **包みは Manifest を持つ**：包みは `CleanedUp` を見たら `finish()`（通常の経路）を呼んで Manifest を取り，**判定の前に**保存する（`Core::Ended`）。`finish(self)` が handle を消費するので（§11 が 2a に残した折り合い），終わった Run は保存した Manifest から答える：`state()` は `CleanedUp { その termination }`，`submit`／`advance_to`／`wait_for`／`run_until_end` は `Err(Ended)`（Kernel が終わった後に返すのと同じ），`events(from)` は Manifest の `events.delivered`，`finish` は保存した Manifest。判定の panic を試験が `catch_unwind` で受けた後も `finish()` は 2 度目の panic なしに Manifest を返し，`Drop` が判定し直すこともない（method の戻りの試験が `Drop` で通らないための形：4 つの method ごとに Run を closure の外に置き，その 1 回の呼び出しの panic を受けて assert する）。`fake` binary の 432 本のうち，終わった後に accessor を読む試験は 0 本（prototype に数える仕掛けを入れて計測）なので，この形は挙動を変えない。
- **宣言は 2 種類**（m2。段 2a で作った形）。v3 の「終わりを期待すると宣言し，検査を外す」は広すぎた：締め切り以外の理由で Run が終わることを期待する所は多く，そこで検査を外すと KC-21a や RS-8a が marker を出さず，偽の kill に戻る。作った宣言は `Declared { end: Option<Expect>, join_is_subject: bool, rx_join_issue_67: bool }`。
  - **期待する終わり** `Expect`：`Refused`（`Failed` が `Validate`／`Prepare`／`Arm`。`Run` 段は不可），`Policy(event の種類)`（`Stopped { cause: Policy { event == その種類 } }`。別の event は不可），`Any`（上限でない終わりは全部。名指しの観測だけの bench の loop 用で，`perf_rx_rate_trial`，`b8_lead_trial`，`run_stage` の 3 fn に限る：走査が止める。上限は `Any` でも失敗で，loop は見た termination を報告し続ける）。外れるのは 3 だけで，2 は外れない。宣言は終わりを足すだけで，ふつうの終わり（`Completed`，`Client`）は消さない。名指したものと違う終わりは 3 で panic する。
  - **上限そのものが主題**：UHD の Run の試験で上限が主題なのは join だけ（3 本：`ur_16_a_wedged_thread_does_not_wedge_stop`，`ur_16_cleanup_keeps_the_streamer_of_a_detached_thread`，`ur_26_the_sending_threads_join_before_the_receiving_ones`。prototype で 432 本を流して KC-12a，KC-21a，RS-8a で落ちた試験は 0 本）。だから `join_is_subject` が外すのは join だけで，RS-8a，KC-21a，KC-12a は主題の試験でも marker を出す。他の上限に宣言は作らない（試験が現れたら足す）。
  - **`rx_join_issue_67`**（段 2a で足した 2 つ目の例外）：§10 の項目 1 の事前の join は，#67（`uhd-rx outlives its 1 s join when Provider::stop meets a timed start queued for a distant origin`）である。origin の遠い受信 segment を持つ Run を origin の前に終えると，`uhd-rx-*` が join の 1 s で detach される（`did not join within 1 s; left detached`）。これを T0 の前後で場合分けして免除する案（`now >= t0`）は採らず（T0 は TX の wedge と #67 を区別できない），join は常に上限である。免除は `uhd-rx-*` の thread の join だけで，RS-8a，KC-21a，KC-12a も `uhd-tx-*`／`uhd-control` の join も免除せず，終わりも宣言しない。宣言 `.rx_join_issue_67()` の場所は 1 行の comment で #67 を引き，#67 を直したら宣言ごと消す。該当は，origin の前に Run を終える健全な 5 本（`ur_13_arm_cases`，`ur_13_arm_compares_the_rate_set_after_the_streamer_opens`，`ur_13_arm_takes_the_rate_set_in_any_order`，`ur_13_a_transmit_stream_is_compared_with_the_whole_device_grid`，`the_candidate_ceiling_admits_what_the_profiles_link_limit_refuses`。5 本とも記録は `uhd-rx-s0` だけ）と，`spec_run_ended` が「refuse されなかった」Run を壊す失敗の経路。mutation の行 P880 だけは `"kill": "bound"` で受ける（kill が #67 の teardown だけで，3 か所に宣言を足すより #67 と一緒に消える宣言のほうが小さい）。新しい issue は立てない。
- (v3.1) **RS-8a の列**（再確認 M2。各段は VERIFIED，負荷で 5 s を超えることは INFERRED で，KC-21a と同じ度合い）：baseline は通る → mutant が負荷の下で走る → テストの `finish()` の中で Provider の stop／cleanup の step が 5 s を超えて捨てられる → `termination.reason` は Stopped／Completed のまま，`did not join within` の行も無い（join を待つ前に step ごと捨てられた）→ step が slot を握ったままなので Manifest に Provider の section が 1 つも入らない（`ending.rs:55-56`：`KC-44: … still held by an abandoned cleanup step`）→ テストの `section(&manifest, "bursts")` が `no section bursts` で panic（`tests/common/mod.rs:560-561`）→ marker なしの `FAILED` → killed。v3 の 2 はこれに当たらなかった。v3.1 では包みの `finish()` が Manifest を返す前に 2 に当たり，marker を出す。
- (v3.1) **残るもの**（段 2a で閉じた）：期待する終わりを宣言したテストが，包みの `finish()` を呼ばずに終わる形は，走査でも `#[must_use]` でもなく **`Drop`** で止める（`#[must_use]` は捨てた値しか見ず，束縛が scope を出る形を見ない。`.finish(` の走査は包みを動かす helper を見えず，`captured`／`finish_spec` で終える Run に false positive が出る）。`Drop` は live の Run を `finish()` して `finish` と同じに判定する（Manifest は捨てる。Kernel の `Drop` が捨てるのと同じ）。この形で終えている試験は今 1 本（`the_candidate_ceiling_admits_what_the_profiles_link_limit_refuses` の `admitted`）で，`admitted.finish()` と書いた。
- **unwind の規則**（段 2a で変えた。以前の「`Drop` は panic せず，unwind の中では `finish()` も呼ばない」を改める）：`Drop` は unwind の中でも live の Run を `finish()` し，判定する。**panic はしない**（`judge` は `std::thread::panicking()` のとき上限の marker だけを書く。unwind 中の `Drop` の panic は process を abort する）。理由は M3 の列：試験の assert が `events()`／`state()` の後で落ち，背景で切れた上限（Policy の終わりの `stop_early` の join の timeout）が marker なしで捨てられるのを止める。Kernel 自身の `Drop for RunHandle` も device-paced の Run を unwind の中で `ending::finish` に通す（`mod.rs:315-320`）ので，abort の元は増えない。`finish()` の panic は Kernel の不変条件が壊れたときだけで，`finish` 自身の `expect`（cleanup が走った後）は `catch_unwind` で受ける（その cleanup が再び走る `debug_assert!` は受けられない：テストでは固定できず，Kernel の変更が要る）。残る限界（受け入れた）：assert が落ちた後の unwind の cleanup の中で切れた上限にも marker が付く（Manifest はもう揃っている）。`finish()` が戻らない形は watchdog の担当。
- (v3.1) **上限が切れる所**（段 2a の後は，呼び出しの中で切れるものは呼び出しの戻りが，背景で切れるものは上の `Drop` と次の検査が受ける）：読んだ Run の上限は 4 つで，どれも包みの呼び出しの中で切れる。KC-12a は生成。KC-21a は 3 か所：batch の dispatch（`pipeline.rs:342`。再確認は「生成」と読んだ。自分では前後 8 行しか読んでいない），`submit`（`:1037`），round の中の agenda の dispatch（`stepping.rs:359-385`：Spec の予定の Action。advance／`wait_for` から届く）。RS-8a は cleanup の中，つまり `settle()` の入った呼び出し（生成，`submit`，advance）か `finish`（`run.rs:584-589`）。join は `finish`。v3 の「どれも（KC-12a は生成，KC-21a は `submit`，join は `finish`）」は RS-8a と，KC-21a の残り 2 か所を落としていた。呼び出しの外で背景に切れる Kernel の上限は，lease の期限（`paced.rs:70-72`。どの生成も attached の lease を渡すので起きない）と，Policy の終わりが走らせる `stop_early` 内の Provider の join だけ（段階 0 の表と M3 の読み。後者は `Drop` と次の検査が受ける）。包みの外で切れる上限は試験側のもの（watchdog，`bounded`，`recv_timeout`）で，切れた所が自分で marker を出す。
- 包みの `finish()` が止まった場合は watchdog が backstop（marker つきの abort）。
- **直結の rig**（`UhdRig`，`Direct`）：上限は rig の中にある（`DeviceAuthority::new` の 5 s，`wait_for_events` の 10 s，`stop` の join）。同じ規則：rig の constructor と method が，上限が切れた所で marker つきで panic する。`unwrap()` をテストに渡さない。
- **end-to-end の試験**（Sol の列）：§6.4 の fixture 3。

### 6.3 marker と判定（Sol v2 minor 1）

- helper は 1 つ：`infra(msg)`。`EZSDR-INFRA: <テスト名>: <msg>\n` を **`std::io::stderr().write_all` 1 回**で書く。libtest の capture は `print!`／`eprint!` だけを取るので，これは capture を通らず，buffer も無い（probe 7 (b)(c)）。その後 panic（包み）か `abort()`（watchdog）。今の watchdog は `eprintln!` の後に abort していて（`watchdog.rs:53-58`），marker は消える（probe 7 (a)）。
- **退けた形**：(1) infra 用の終了コード：runner の子は Cargo で（`mutate.py:97-101`），Cargo は abort でも panic でも 101 を返す（probe 7）。(2) file に書く：sim の container と shard ごとに path を通す配線が要る。stderr で足りる。
- **判定は出力の行で決める**（終了コードと signal は Cargo 越しには読めない）：

| 出力 | `assert` の行 | `"kill": "bound"` の行 |
|---|---|---|
| `test result: FAILED`（Python は `FAILED (`）があり，`EZSDR-INFRA:` も Cargo の `(signal:` も無い | **killed** | killed |
| `EZSDR-INFRA:` がある（`FAILED` の有無に依らない） | `ERROR` | `killed (bound)` |
| mutate.py の `TEST_TIMEOUT` | `ERROR` | `killed (bound)` |
| `(signal:` があり marker が無い（OOM，segfault），または失敗したのに `test result:` の行が無い | `ERROR` | `ERROR` |
| build の timeout | `ERROR` | `ERROR` |

- marker はテストに結びつかない。filter が複数のテストを選び，1 本が assert で，別の 1 本が上限で落ちたら `ERROR` になる（安全側。その行を流し直す）。
- `ERROR` は `gate.py` が broken として落とす（`gate.py:189-193`）。結果は固定の欄に出す（#91：行の名前に `: killed` があると読み違える）。
- **marker は失敗を通過に変えない。** 変わるのは mutation の結果（killed → `ERROR`）だけで，gate の test の段では marker つきでも失敗である。
- `bound` の行は負荷による偽の kill が残る。`gate.json` に数と id を出す。直近の全 list（2 115 行）で `killed (timeout)` は 2 行（G16，E2A2A-01）。watchdog や 10 s の上限で落ちた行は今はただの `killed` なので，本当の数は M 段の最初の全 list で分かる。

### 6.4 Cargo を通した試験（gate の段，mutant なし）

`mutate.py --e2e`：`fake` binary の `infra_fixture_*`（環境変数 `EZSDR_INFRA_FIXTURE=<名前>` が無ければすぐ戻るテスト。`#[ignore]` にはしない：実機テストの規則と混ざる）を，行を流すのと**同じ `cargo_test()`** で流し，判定を固定する。gate は mutation の image の中で走るので，probe 7 の INFERRED（image での挙動）はここで消える。

| # | fixture | 期待する出力 | 判定 |
|---|---|---|---|
| 1 | assert の失敗 | `FAILED`，marker なし | killed |
| 2 | 止まった時計＋`EZSDR_TEST_WATCHDOG_SECS=1`（`watchdog.rs:42`） | marker，`(signal:`，`test result:` なし | `ERROR`／`bound` の行なら `killed (bound)` |
| 3 | **Sol の列**：prepare が戻らない device で包みを生成（KC-12a，5 s） | marker 1 つ，`FAILED`，`panicked at` は 1 回 | `ERROR` |
| 4 | `submit_limit`（2a）：外部 clock の `fake_real` を `Stalls`（下）で包み，Running の後，uhd-control が読む `ref_locked` を 8 s 止めて `submit`（KC-21a）→ 後ろに `panic!` | marker `a submit met KC-21a`，後ろの文は出ない | `ERROR` |
| 4b | `agenda_limit`（2a）：4 と同じ `Stalls` で，Spec の予定に **target つきの `stop`**（offset 50 000 tick）を置き，`advance_to`（+3 s）で T0 と stop の時刻を越える（`stepping.rs:359-385`）→ 後ろに `panic!`。Spec の `tx_burst` は agenda に来ず（時刻つきの template は `Running` に入るとき 1 度に dispatch され，KC-21a の 1 か所目に当たる），untimed の `stop` だけが agenda に来る | marker `an advance met KC-21a`，後ろの文は出ない | `ERROR` |
| 5 | marker なしの `abort()` | `(signal:` | `ERROR`（`bound` でも） |
| 6 | `cleanup_limit`（2a）：手動 clock の `Stalls` で `close_streams` を 7 s 止め，T0 の後にテストが `finish()` を呼び，後ろで `section(…)` を読む（RS-8a，5 s） | marker `the Run met RS-8a`（`finish` の中），`no section` の文は出ない | `ERROR` |
| 7 | `declared_end_cleanup_limit`（2a）：手動 clock の `Stalls` に `FakeFault::Unlocked`（2.3 s），Spec は外部 clock，**`Policy("radio.CLOCK_LOST")` を宣言**して `run_until_end`（+5 s），6 と同じ cleanup の step が戻らない。終わりは DEVICE_LOST でなく CLOCK_LOST（lost の device は `close_streams` を飛ばすので，cleanup の step が止まらない：`provider/mod.rs` の `cleanup`） | marker `a wait met RS-8a`（`run_until_end` の戻りで。宣言は 2 を外さない），後ろの `section` の文は出ない | `ERROR` |
| 8 | `join_at_finish`（2a。note 51 に無かった行）：`Gated`（実時間）で Provider の 2 つの thread を device の呼び出しに止め，宣言なしで `finish()` | marker `the Run met join`（`uhd-tx-s0`，`uhd-rx-s0`）。join を `finish` で見る J317／J318 の機構の，Cargo 越しの唯一の覆い | `ERROR` |

3・4 の device（prepare／Action を終えない FakeDevice の fault）が今あるかは見ていない（INFERRED：`Gated` の形で作れる）。**段 2a で作った形**：4・4b・6・7 は `infra_fixtures/mod.rs` の test 局所の device `Stalls`（FakeDevice を転送する包み。`fake.rs` の 8 つと同じ形）を共有する：`ref_locked` は armed のとき 8 s，`close_streams` は armed のとき 7 s 眠り，眠る前に channel へ名前を送る。4 と 4b は `await_entry`（`recv_timeout` 5 s。切れたら `EZSDR-INFRA: fixture setup …` を書いて panic）で「stall に入った」ことを見てから進む（固定の 700 ms は使わない。Sol の minor）。6 と 7 は試験の呼び出しの中で stall に入るので handshake が要らず，stall に入らなければ `has` の pattern が合わない。8 は setup を `setup()` で包み，setup の panic は `EZSDR-INFRA: fixture setup … did not complete` の marker つきになる（sentinel の assert が kill に読めるのを防ぐ）。note 51 の INFERRED の 2 つの形はどちらも細部が違った：`Gated`＋手動 clock は KC-21a の 3 か所目に届かない（待つ participant が round の前に clock を止める），Spec の `tx_burst` も届かない。各 fixture は `infra_fixture_<名前>`（`EZSDR_INFRA_FIXTURE` が名指さなければすぐ戻る。mutation の filter ではない）。加えて `mutate.py` の self-test（文字列だけ：上の表の全部の升）。

(v3.1) **KC-21a の 3 か所と fixture**（m1）：`submit`（`pipeline.rs:1037`）は fixture 4，advance／`wait_for`（`stepping.rs:384`）は 4b。3 つ目（`pipeline.rs:342`）に専用の fixture は置かない：どの戻りで切れても，走る検査は fixture 3・4・4b と同じコードで，違うのは理由の接頭辞と戻る method だけであり，接頭辞の分類は純関数の test（`KC-12a:`，`KC-21a:`，`timed_out`，`did not join within`，それ以外）で固定する。段の割り当て：1・2・3・5 は M，4・4b・6・7・8 は 2a（包みの method と `finish()` の検査が要る）。6・7 の device（cleanup の step が戻らない fault）は `Stalls`（上）で作れた（V：各 fixture を 20 回，marker 1，`panicked at` 1，sentinel なし）。4b の形は上のとおり target つきの Spec `stop` に変わった。`mutate.py --e2e` の case は 9 から 14 になり（`4-submit-limit`，`4b-agenda-limit`，`6-cleanup-limit`，`7-declared-end-cleanup-limit`，`8-join-at-finish`），`e2e_case` は `has` に加えて「合ってはいけない pattern」`lacks`（sentinel）を読む（判定は純関数 `e2e_seen`。self-test に `lacks` の升を足した）。gate の e2e 段は `e2e: 14/14`。

### 6.5 残りの規則（v2 から変更なし）

- **「来ないこと」で殺す行**：A の rig の `wait_for_events` は device の時計を 1 ms ずつ進めている（`parity.rs@fe8:454`）。device の時刻での上限（「T から D たっても来ない」）を先に置けば負荷に依らない assertion になる。実時間の 10 s は backstop（marker）。Run の `wait_for` は元から device の時刻の horizon。流していない。
- **filter を，選ばれるテスト全部に対して解く**：libtest の filter は module path を含む名前の部分文字列（probe 7 (d) で VERIFIED）。例：`kc_36_`（phase7 C07）は `device_paced.rs` の 8 本を選ぶ。
  - 印は `FakeDevice::new` 自身が出す（clock が Real で，mutate.py が baseline に立てる環境変数があるときだけ `EZSDR-CLASS: real-clock`。`eprintln!` でよい）。baseline を `--show-output` つきで流して読む（probe 7 (e) で VERIFIED）。
  - `FakeConfig::default()` は Real（`device.rs:415`）で，`fake_real` を通らずに Real を作る所が 66 か所ある（grep の上限。1 本ずつは見ていない）。
  - M 段では報告だけ（`gate.json` に `real-clock (unclassified)` の数と id）。段 3 で 1 本ずつ：Manual にして印を消すか，行を `bound` と宣言。その後は，印があって宣言の無い行は broken。
  - 今分かっている下限：`fake.rs` の本体に `fake_real(`／`ClockMode::Real` と書いた 4 本を 9 行（J295，P1120〜P1123，P1149，P1150，P1156，P1157）が選ぶ。
- **規則 M-R**：遅れ・phase・受け取りまでの遅れを kill の証拠にする行は，直結の rig か unit のテストを filter にする。予行は filter にしない（落ちても `ERROR`）。

### 6.6 3 strikes の問い（#91 は `mech:failure-collapsed-into-value` の 8 件目）

failure-mechanisms.md:94 の「既にある note の答えの局所的な適用」に同意する。概念は消せない（結果は 3 値：証拠つきの kill／survivor／判定不能）。今の型は「それ以外＝killed」という残りの枝で失敗を値に潰している（`mutate.py:146`）。結果の parser は `remote_mutate.py` の 1 つを `gate.py` が import している（`gate.py:49`）ので二重ではない。新しい design note は要らない。

## 7. 調査と段階

### 7.1 変わるテスト（grep の件数。1 本ずつの分類は段階 0）

| ファイル | 数 | 形 | 段 |
|---|---|---|---|
| `ezsdr-acceptance/tests/parity.rs` | (v3.1) `1371ffa` で 49 本（`grep -c '#\[test\]'`），`wait_until` のある行は 52（`grep -c wait_until`）。v3 の「47 本，51 か所」は `fe86629` の数。#96（`aa478bc`）が 8 本を手順で直し，**受け取りの窓の 14 本**をこの設計に預けた（`plan/phase8/reviews/n96-dispositions.md:10`。名前は実装者の report `~/.cache/ezsdr-fixes/2026-11-11-n96/report.md:57-62`。14 本とも main にあることは grep で確かめた）：`s15_parity_booking_rule_both_receipt_orders`，`s15_parity_b_first_accepted_booking_rule`，`s1_parity_short_burst_front_s_s3_s5`，`s1_parity_stop_during_padded_device_burst`，`s1_parity_padded_front_survives_normal_close`，`s1_parity_short_head_region_is_end_plus_gap`，`f_parity_two_transmit_streams_front_at_their_own_gap`，`s1_parity_grouped_and_split_fronts`，`s1_parity_short_burst_then_adjacent_burst_before_and_after_dispatch`，`s3_parity_booking_after_a_closed_short_burst_follows_d5`，`s3_parity_booking_after_a_device_burst_s_short_tail_follows_d5`，`s5_parity_b_received_before_a_refuses_a`，`s13_parity_b_refused_at_receipt_and_the_value_answers`，`s14_transmit_values_on_an_empty_stream_decide_the_same_on_both_providers` | S1，直結 | 1b。rig だけ変える。#96 の直しは残してよい |
| `ezsdr-radio-uhd/tests/fake.rs` の `Direct` | 41 本（`Direct::` を呼ぶ 38 と `refused_second_start_of_stream_1` 経由の 3） | S1，直結 | 1c。tests/README の例外が消える（済）。`Gated` を使う 2 本は実時間の fake で，`drive()` は何も登録しない |
| `src/provider/{tx,rx}.rs`，`tx/tests/{e1,e2b,o1}.rs` | `drop(frozen)` 17，`Frozen` 約 75 | S1（`drive()` の `handed_over`） | 1d（G の 4，W2 の 3），4（残り） |
| `fake` binary の Run を通すテスト | 398 本中，注入で 11 本，うち予行 6 本 | S1，Run | 2a〜2c |
| `ezsdr-acceptance/tests/uhd.rs`，`ezsdr-server/tests/protocol.rs` | 17 本（注入で 1 本），74 本（0 本） | S1，Run | 2a・2b |
| Real の FakeDevice | `fake_real` ほか明示 12 か所，default の Real 66 か所（上限） | S2 | 3 |
| `ezsdr-kernel/tests/device_paced.rs` | 12 本（ms の上限 4，1 s の上限 7，「100 ms に 50 block」1）。6 本が phase7 の G05・G08・G36・C05・C06・C07 の filter | S3 | 3 |
| `python/tests/test_easy_api.py`，`ezsdr-server/tests/watch.rs` | 上限 10 s，`recv_timeout(BOUND)` | P3 に合う（INFERRED） | 変更なし |

### 7.2 段階（どの段の終わりも tree は green）

| 段 | 中身 | 危険度 |
|---|---|---|
| 0 | 表だけ：W 表（駆動役の時計以外の待ち），R 表（Run のテストごとの級，証人，`receipts` が要るか），実時間の上限の一覧（どこで切れ，どの rig の口が marker にするか，背景で切れるものが無いこと。(v3.1) **RS-8a を含める**：step ごとの deadline，Provider が自分の deadline を宣言しているか（`run.rs:538`），KC-21a の 3 か所），timing を理由にした `Inconclusive` の一覧。(v3.1) R 表には「遅い L が原因で窓の中の Action に fault が出る trial」の列を足す（§5.3） | なし（文書）。re-check reviewer |
| M | §6.3〜6.5：`mutate.py` の出力による判定と固定の欄，`infra()` と watchdog の書き方，`bound` の宣言，印（報告だけ），`--e2e` の fixture 1・2・3・5，**生成時の分類**（`session_with` と `UhdRig` の constructor だけ：包みを待たずに入る。(v3.1) 生成の戻りで `CleanedUp` なら Manifest を取り，理由の接頭辞と `cleanup_failures` の `timed_out`（RS-8a）の両方を見る。`session_with` 以外の生成 12 か所（§5.4 の一覧）は M では入らない），self-test。全 list を 1 回流し，`ERROR` の行を 1 本ずつ決める | risky ではない（tooling）。gate の意味が変わるので Sol の code review |
| 1a | `host.rs`：時計ごとの登録，`drive()`，7 本と rows。`Provider::join`。UR-33 の M1 の文 | **risky**（参加者の同一性，lifecycle） |
| 1b | (v3.1) `UhdRig`：guard を最初の field，登録は Authority の後，`TxBlocks` の assert。failing test first の集合は，#96 がこの設計に預けた**受け取りの窓の 14 本**（§7.1）：試験のスレッドへの遅延の注入で今は落ち，1b の後は通ること。probe 1c の 7 本のうち #96 が手順で直したものは，もう落ちないので集合に入れない | **risky** |
| 1c | `Direct`：W 表のとおり，待ちの移行と登録を同じ commit（作業枝では A 待ちと上限の移行，B 登録，C guard の共有，の順で，各 sub-step が緑）。failing test first の集合は既存の 1 本，`ur_25_a_receive_cut_counts_its_own_stream_s_packet_size`（`push` の前に実時間の遅延を注入すると落ちる）。`Gated` の実時間の待ちは 3 へ（D は見送り） | **risky** |
| 1d | G の 4 か所を guard（`handed_over`），W2 の 3 か所は書き直して guard（済：下の「段 1d の後」）。`join` する helper が残る 3 か所（`tx.rs` の `-deadline`）は W2 の形で `Frozen` が無いので 4 へ | **risky**（小さい） |
| 2a | (v3.1) Run の rig の包み（共有 trial と `hardware.rs` を含む。`session_with` 以外の生成 12 か所（§5.4 の一覧）も包みの生成に替える），**全 method の検査（§6.2：上限の分類は常に，宣言は 2 種類）と fixture 4・4b・6・7（と 8）**，診断の注入点，`protocol.rs` の `handle`（82 か所を 1 fn に通す），走査（`connect(`／`start_spec_run(`／`RunHandle` を含む）。**作業枝で実装済み**（8 sub-step：包みと純関数，切り替え，acceptance，`protocol.rs`，fixture，走査，注入と sweep，文書。結果は下の「段 2a の後」。gate は orchestrator が閉じる） | **risky**（fake と実機の境界）。audit 表。compile は gate，挙動は次の bench |
| 2b | R 表のとおり R1・R2 を直結へ移す／assert を外す。rows の filter を付け替え，同じ mutant が死ぬことを確かめる | 場面ごと |
| 2c | 遅れの証拠の純関数 `iii::late_evidence` と印を書く `Rehearsal`，試験側の literal と本番の定数の一致の test，予行 9 本の trial と assert の組み替え（trial の記録と判定の分離，admit の assert を判定の後へ），held-tail の C = 1 000，`stop_next_trial` の N2（実機は N2 の行を 1 本足す），`stop_booking_verdict` の fault 先行，`long_close` の形の欠けを FAIL に，`mutate.py` の baseline を `judge` に。**production と Manifest は変えない**。**risky**：実機と共有する trial と判定。audit 表，fidelity の review，次の bench |
| 3 | S2：印のついたテストを 1 本ずつ。S3：Kernel の 12 本（Q5） | 場面ごと |
| 4 | `Frozen`／`released` の約 75 か所。分類表から（Q6） | 場面ごと |

(v3.1) M と 2a の間の窓（2a の後は閉じた）：Run の途中で KC-21a が切れて後ろの assert で落ちる形，`finish()` の中で RS-8a が切れて section の読みで落ちる形，`session_with` 以外の生成 12 か所（§5.4 の一覧）の生成時の上限は，2a まで偽の kill のままだった（§8）。文書（UR-33，tests/README，failure-mechanisms，note 37 §4 に 1 行）は，それを偽にする段の diff に入れる。

## 8. 正直な一覧

| 対象 | rig | 状態 | 条件・残るもの |
|---|---|---|---|
| S1 | 直結（`UhdRig`，`Direct`，unit の G と W2） | **構造で閉じる**（1b〜1d の後。`Direct` は 1c で閉じ，`Gated` の 2 本は実時間で閉じない） | テストが rig の待ちだけを使うこと。破れば毎回 hang し，marker つきで**検出** |
| S1 | 直結，rig が 2 つ | 構造で閉じる | 2 つは同時に進まない。根拠は 1a の unit test |
| S1 | Run，R1・R2 を移したもの | 構造で閉じる（直結として） | 段階 0 の R 表が正しいこと |
| S1 | Run，R0 | 該当しない | R0 の判定は review。sweep は見張りで証明ではない |
| S1 | Run，実機と共有する予行 | **閉じない。囲う** | 負荷で落ち得る。§5.3b の遅れの証拠がある trial は判定せず印を書き，単独で 2 回まで流し直す（3 回とも印なら gate は赤）。証拠の無い trial はそのまま判定する。capture は `LogEntry.time`（片側の証人：submit の中で遅れた capture は判定される，N5） |
| 遅れの証拠 | 予行（fake） | **検出はするが kill にしない** | Action を遅らせる変異は印が 3 回出て `ERROR`。kill は直結／unit の filter（M-R）。今ある 6 行の filter の予行は，遅れとは別の理由で kill される（2c の計画 §6.2） |
| 実機 | 実機 | **遅れは変えない** | 遅れた trial は今どおり INCONCLUSIVE（`late_arrival_verdict` ほか）。変わるのは，遅れの報告が無いのに長い close の形が欠けた trial が FAIL になることだけ（M4）。Q15 は 2c で作らない（decision 3） |
| S1 | `protocol.rs`，Python | **対象外** | 注入点を 1 か所にするだけ |
| S2 | — | **囲う**（段 3 の後） | 印は constructor が出す。行は `bound` と宣言して数える。default の Real 66 か所は未分類 |
| S3 | Kernel `device_paced.rs` | **書き直す**（段 3）＋走査 | 残る秒の上限は下の「包みの無い rig」 |
| 偽の kill：assertion が証拠 | 直結 | 閉じる（S1 について） | — |
| 偽の kill：mutate.py の timeout，build，signal，watchdog | 全部 | **検出**（M の後） | watchdog の marker は §6.4 の fixture 2 が根拠 |
| 偽の kill：生成時の上限（KC-12a，生成の中の KC-21a と RS-8a，Authority の 5 s） | (v3.1) **`session_with` を通る生成と `UhdRig` だけ** | **検出**（M の後） | fixture 3 |
| 偽の kill：生成時の上限 | (v3.1) `session_with` 以外の生成 12 か所（§5.4 の一覧）：`ezsdr-acceptance/tests/uhd.rs:51, :66, :264`，`tests/common/measurement.rs:1904` など | **検出**（2a の後。済：`Run::connect`／`Run::start_spec` に替えた）。M〜2a の間は**対象外**だった | 走査が raw の生成と `RunHandle` を `run.rs` の外で止める（§5.4）。`uhd.rs` も `run.rs` を取り込む |
| 偽の kill：Run の途中の上限（KC-21a の `submit` と advance），**cleanup の上限（RS-8a）**，join | (v3.1) 包みを通る uhd の Run の rig（`fake.rs`，`tests/common`，`uhd.rs`。§5.4 の 12 か所は 2a で包みに替えた） | **検出**（2a の後。済）。M〜2a の間は**対象外**だった | 検査されるのは，Run を終わらせ得る呼び出し（`connect`，`start_spec`，`submit`，`advance_to`，`wait_for`，`run_until_end`）の戻りと `finish` と `Drop` で，**read-only の method の戻りは検査しない**。背景で切れた上限は次の検査される呼び出し・`finish`・`Drop` で見える（§6.2）。RS-8a は Manifest を読む所でしか分からないが，包みが Manifest を取って判定し，`finish()` を呼ばずに離した Run は `Drop` が判定する。負荷で 5 s を超えるかは INFERRED。**限界**：fixture の無い形（KC-21a の 3 か所目 `pipeline.rs:342`）は同じ検査が走るという理由だけで覆う（§6.4）。`#[ignore]` の bench の Run は包みを通るが，挙動は次の bench で確かめる |
| 偽の kill：上限 | `protocol.rs`（`Server` 越し） | **対象外**（段階 0 で，返事から Run の終わり方が読めるか確かめる） | — |
| 偽の kill：上限 | Kernel の test（`device_paced.rs` ほか），Python | **対象外** | 包みが無い。上限が主題の行は `bound` と宣言 |
| 偽の kill：marker を出さない本番の内部の上限 | Authority の `CLOCK_JOIN` の detach，Kernel `state.rs:65` | **対象外** | — |
| 偽の kill：`bound` の行 | — | **対象外**（宣言して数えるだけ） | — |
| 偽の kill：予行が負荷で遅れる | Run | 証拠あり → 印 → `ERROR`。証拠なし → そのまま判定，FAIL は **killed と数える**（§5.3b） | 遅れた回の中の別の Action の故障は，その回では捕まえない（単独の流し直しで再現すれば捕まる）。負荷の下でだけ遅れる regression は `gate.json` の timing の流し直しの数でしか見えない。submit の中で遅れた capture は判定される（N5：`LogEntry.time` は片側の証人，INFERRED） |

## 9. 既存の約束との照合

| 約束 | 影響 |
|---|---|
| 計画 30 M1 | **改める**：駆動役を足す。M2〜M7 は変えない |
| 計画 30 §3（試験が時刻を進める案の却下） | 保つ。Kernel に何も足さない |
| 計画 30 §4.3（`JOIN_BOUND` は実時間） | 保つ。待ち方だけ時計の待ち |
| 計画 30 D1（smoke set は「実時間に追いつく」を証明しない） | 同じ理想化を駆動役に広げる。envelope は緩めない。**駆動 rig は実機に無い挙動（駆動役の遅れが時刻に現れない）で通る**ので，示すのは受け取りの時刻を決めたときの規則だけ |
| note 37 §2 の規則，§4（`hold()` の却下），決定 4（`Direct` の例外） | 規則は Run の rig と実機に残る。§4 は改める（probe 2・4）。決定 4 は 1c で不要になった（`Direct` も登録する） |
| AGENTS §9 実機テストの予行 | 規則は変わらない。**予行は決定的にならない**（Run の rig のまま）。遅れの証拠で囲う（§5.3b） |
| `tests/common/mod.rs:534-556`（timing の前提の失敗は test の失敗のまま） | 保つ。`require_stream_timing` の失敗は失敗のまま。差し控えるのは予行の trial だけ |
| `tone.rs:19`（INCONCLUSIVE は通す） | 実機では保つ。fake の予行では，遅れの証拠がある trial は INCONCLUSIVE でなく印つきの失敗（単独の流し直しで通れば通る：§5.3b） |
| AGENTS §3（Kernel は小さく） | Kernel の変更なし。`receipts` は作らない（§5.3b） |
| KNOWN_LIMITATIONS，EA，RM | 試験の時刻に触れる記述は無い（`grep -n -i "manual clock\|test thread\|participant" KNOWN_LIMITATIONS.md design/16-easy-api.md design/07-radio-model.md`：0 件）。`receipts` は作らない |
| design/18 UR-33 | M1 の文を改める（実装の diff で） |

## 10. owner の判断

決まったもの：Q1〜Q6 と (i)（冒頭の表）。(v3.1) Q4 は済んだ（`aa478bc`）。残り（番号は v2 のまま。**★ は推奨か文が変わったもの**。(v3.1) Q16・Q17 は v3.1 で足した）：

- **Q7 同じ時計への二重登録**：拒否（panic）か refcount か。推奨：**拒否**（変わらず）。拒否は登録を作る前（§4.1）。
- **Q8 上限で落ちたときの扱い**：(a) 行が `"kill": "bound"` と宣言，(b) その場で流し直す，(c) 両方。推奨：**(a)**（変わらず）。判定は出力の行で決める（§6.3）。
- **Q9 実時間のテストの見分け方**：(a) `FakeDevice::new` が出す印を baseline の出力で見る，(b) 名前の表。推奨：**(a)**（変わらず）。`--show-output` と module path の filter は probe 7 で確かめた。
- **Q10 受け取りの記録（2026-11-13 取り下げ：2c は Manifest を変えない。以下は当時の文）**：UHD Module の Manifest に section `receipts`（Action ごとの受け取りの root tick）を足すか。本番の変更（Extension の層）。推奨：**足す**（v2 は「証人が作れない phase があれば」）。(i) は受け取りの記録が無いと実装できない：今の記録は受け取りの遅れと dispatch の遅れを分けられない（§5.3）。足さないなら，予行に timing の INCONCLUSIVE は無くなり，遅れは全部 FAIL になる（負荷で落ちた予行は kill に数えられる）。頼るのは R1-tx と R2 の予行（読んだ範囲で `n47s5_*` の 6 本の trial）。
- **Q11 実機と共有する trial の包み**：(a) 実機のコードも包みに替える，(b) raw `RunHandle` を渡し続ける。推奨：**(a)**（変わらず。理由が増えた：§6.2 の検査は包みにしか置けない）。
- **Q12 (b′)**：採らない（変わらず）。
- **★ Q13 段 M を最初に置く**：推奨：**置く**（順序は変わらず）。ただし「以後の gate が正直になる」は言い過ぎだった。(v3.1) M の後に正直になるのは timeout，build，signal，watchdog と，`session_with`・`UhdRig` を通る生成の上限まで。`session_with` 以外の生成 12 か所（§5.4 の一覧），Run の途中の上限（KC-21a），cleanup の上限（RS-8a），join は 2a の後，包みの無い rig は対象外（§8）。「直結の rig が gate を落としている」は #96 の後も成り立つ（14 本が残っている：§7.1）。2a を 1a より前に置けば窓は縮むが，今 gate を落としている直結の rig（#96 の型）の修正が遅れる。推奨は今の順。（2a は作業枝で実装済み：下の「段 2a の後」。）
- **★ Q14 規則 M-R**：推奨：**採る**。範囲：遅れ・phase を殺す行は直結か unit を filter にする。予行で遅れを作る変異は `ERROR`（印が 3 回：§5.3b）。
- **Q15（新。2026-11-13 前提が変わり 2c では作らない：(i) が取り消され，実機に遅れの新しい INCONCLUSIVE の出どころが無い。F3（Session の Stop の case が実機で毎回 INCONCLUSIVE）は `stop_next_trial` の N2 で直す）実機で conclusive な trial が 0 の case**：timing の INCONCLUSIVE が全 trial で続いた case を「未実施」として FAIL にするか。推奨：**する**。(i) の残り（受け取りまでの regression）が実機で通り続けるのを止める唯一の手で，1 行の規則で済む。今の `hardware.rs` に同じ規則が既にあるかは見ていない（(v3.1) 再確認が `hardware.rs` と `tests/common` を `conclusive` などで grep した範囲では，見つからなかった）。
- **(v3.1) Q16（新。再確認 m7。2026-11-13：b″ のとおり単独で 2 回まで流し直す。3 回とも印なら gate は赤，mutation は `ERROR`）fake の予行で timing の INCONCLUSIVE が出たとき**：(i) が決めたのは trial の判定で，gate がどう振る舞うかは決めていない。
  - (v3.1) (a) **marker つきで落とす**（v3 の §5.3）。コードは足さない。負荷の下では予行が gate を赤くする（test の段は marker つきでも失敗）。gate の流し直しが要る。
  - (v3.1) (b) **新しい Run で K 回までやり直し，全部遅いときだけ marker つきで落とす**。判定に使うのは，受け取りが窓の中に入った回の結果である。
  - (v3.1) 推奨：**(b)，K = 3**。理由：今の fake の予行は timing の INCONCLUSIVE を通している（10 か所が「`Fail` でなければよい」）ので，(a) は負荷の下の gate に**新しい赤**を足す。(b) は赤を足さず，予行の力も落とさない：やり直すのは 5（遅い Action 自身の証拠しか無い trial）だけで，FAIL（6）は 1 回目でそのまま失敗になり，やり直さない。
  - (v3.1) §2 の F（流し直す）と Q8 (b) を退けたこととの違い：あれは**失敗した**テストや行を流し直す案で，regression を確率で隠す。(b) がやり直すのは，受け取りの記録が「場面を作れなかった」と独立に示した trial だけ（P7）で，実機が INCONCLUSIVE の trial を数えずに次の trial へ進むのと同じ扱いである。
  - (v3.1) 条件：わざと遅く受け取らせる case（`fake.rs:7926-7936`）はやり直さない（INCONCLUSIVE が期待値）。やり直した回数は出力の行に出し（`EZSDR-RETRY: <テスト名>: <n>`），`gate.json` で数える：黙ってやり直さない。mutation では変わらない：受け取りを毎回遅らせる mutant は K 回とも遅く，marker → `ERROR`（M-R の行が殺す）。
  - (v3.1) (b) が失うもの：受け取りを**ときどき**遅らせる regression は，やり直しで通る（(a) なら marker つきで赤くなり，人が見る）。(i) の残りと同じ型で，M-R の行と `EZSDR-RETRY` の数で見る。
  - (v3.1) 確かめていないこと：負荷の下で予行の受け取りがどれだけ遅れるかは測っていない（K = 3 は根拠のある数ではない）。trial を新しい Run で作り直す費用は trial ごとに読んでいない。小さく始めるなら (a) で入れ，gate が実際に赤くなったら (b) を足す，でもよい。
- **(v3.1) Q17（新。再確認 m5。2026-11-13：Kernel の log の片側の証人のまま，Provider に宛てた Action の `receipts` の節は削除）R1-k の証人**：Kernel が受け取る Action（capture）の「受け取りが遅い」を，Kernel の action log の `LogEntry.time` で決めてよいか。(i) の文は「Q10 の受け取りの記録から」で，Kernel の action log はその外にある。推奨：**認める**。P7 の「受け取った側が残した記録」に当たり（capture を受け取るのは Kernel），今ある記録で，本番の変更が要らない。認めないなら，capture の遅れに timing の INCONCLUSIVE は無く，遅い capture は全部 6 で判定する（負荷で落ちた予行は FAIL になる）。確認が出るまで v3.1 は後者で書いてある（§5.3 の表）。注意：§5.3 で退けた「Kernel の admit の時刻が窓の中なら，受け取りが遅くても FAIL」とは別の話で，あれは Provider に宛てた Action に Kernel の時刻を使う案である。遅い capture を Kernel がどう扱うかは読んでいない（INFERRED）。

## 11. 確かめていないこと

- A の登録（時計ごと）は実装も probe もしていない。`Direct` と，その試験が使う fault つきの FakeConfig 2 つは 1c で VERIFIED（41 本が登録の下で通る）。W2 の書き直しは 1d で VERIFIED（3 か所 9 本が通り，20 回 0 失敗）。
- R 表は無い。読んだ trial は `n47s5.rs` と `fake.rs` の 4 本だけ。`n47`，`d9`，`measurement` の trial，`tests/common/` の 142 本，probe 6 で新しく落ちた 2 本は読んでいない。
- `receipts` は作らない（§5.3b）。遅い capture を Kernel がどう扱うか（R1-k）は読んでいない（INFERRED）。
- (v3.1) §6.2：背景で切れる Kernel の上限が無いことは INFERRED（段階 0 の表：lease の期限だけ。Policy の終わりの `stop_early` 内の join は M3 の読みで，2a の `Drop` が受ける。流した試験は無い）。`protocol.rs` の生成の経路は読んでいない。（v3 がここに置いていた 2 つは消えた：`ending::request` は state を変えず，`settle()` が戻りの前に `CleanedUp` まで進める。`finish()` は `CleanedUp` の後は何もしない。`uhd.rs` の生成は読んだ：§5.4。）
- (v3.1) RS-8a の step が負荷の下で実際に 5 s を超えるかは INFERRED（KC-21a と同じ度合い）。UHD の Provider が cleanup の deadline を自分で宣言しているか（`run.rs:538` の既定を上書きするか）は読んでいない。
- (v3.1) M1 の列（C が遅く A に underflow）と，遅い L が原因で窓の中の Action に fault が出る列は，テストとして流していない。
- (v3.1) 包みが `CleanedUp` の後に Manifest を取ったあとも `events` などの method を使えるようにする形：2a で決めて作った（包みが Manifest を保存して判定する：§6.2）。
- (v3.1) v3.1 では build，probe，mutation を流していない。`settle()` の行番号（`ending.rs:15-47` ほか）は再確認の VERIFIED を引いたもので，自分では読み直していない。
- §6.4 の fixture 3 の device は M で作った。4・4b・6・7・8 の `Stalls` は 2a で作って流した（§6.4）。
- probe 7 は local の cargo 1.99.1，リポジトリの外の crate。mutation の image では流していない。
- probe は各設定 1 回。mutation は流していない。`wait_for_events` の device の時刻の上限は流していない。
- わざと遅く受け取らせる予行（`fake.rs:7926-7936`）が負荷に依らず遅い側に留まることは INFERRED。
- 「66 か所」「9 行」「10 か所／2 か所の assert」は grep の数で，1 本ずつは見ていない。

## 12. Sol の指摘の対応

| review | 指摘 | 扱い |
|---|---|---|
| v1 major 1（1c-i は hang を作る） | 採用 | §4.4，段階 0 の W 表 |
| v1 major 2（実時間の注入は device の遅れを保証しない） | 採用 | 注入は診断（§5.4） |
| v1 major 3（走査は境界を守れない） | 採用 | 包み＋構文の走査（§5.4） |
| v1 major 4（mutation の正直さ） | 採用 | §6 |
| v2 major 1（`finish()` の前の失敗） | 採用 | §6.2，fixture 3・4，§8 と Q13 の文 |
| v2 major 2（外した phase は infra の証拠ではない） | 採用，owner (i) | §5.3，P7，Q10，Q15 |
| v2 minor 1（abort と capture） | 採用，probe 7 で VERIFIED | §6.3，§6.4 |
| v2 minor 2（v1 §7 の古い保証） | 採用 | §9。この文書は v1・v2 に頼らない |
| v2 note（二重登録は作る前に拒否） | 採用 | §4.1，test (3) |
| v2 note（Q9：環境変数を読むこと自体は欠陥でない。既定は off，protocol の stdout に出さない） | 採用 | §6.5：印は環境変数があるときだけ，stderr |

退けた指摘は無い。

## 付録：owner の決定の記録（Q1〜Q17，(i)）

## Owner decisions

1. Q1 = A (owner 2026-11-11): the thread that drives a rig becomes a participant of the manual clock, enlisted by the rig's constructor; first stage. The owner was told that only S1 in Provider-direct rigs is closed by construction; S1 in Run rigs is detected deterministically, S2 contained, S3 rewritten and scanned.
2. Q2 = yes (owner 2026-11-11): `Provider::join`'s 1 ms wait becomes `host.wait` (a clock wait that tells the manual clock the joining thread is idle); `JOIN_BOUND` stays real time. On the real clock it is the same 1 ms wait, except that a `notify_all` may end it early (harmless: the loop re-checks).
3. Q3 = (a) (owner 2026-11-11): Run-based rigs get an always-on real-time delay at every rig call that returns control to the test (reading time, after advancing, after waiting for events) plus a governance scan, so a violating test fails on its first run. Conditions the owner accepted as part of the decision: (i) the scan covers by crate/directory so new test files are included automatically, and forbids bypassing the rig's entry points (`.now(`, `Instant::now`, `thread::sleep`, …); (ii) the delay is longer than the largest lead a test relies on, set in one place; (iii) every rig call that returns control to the test is covered (Sol to check for gaps). Genuine races inside Provider/Kernel code are product bugs, outside this scheme. (b) only if some of the failing tests cannot be fixed.
4. Q4 = finish #96 first (owner 2026-11-11): the per-test fix on `work/n96` (with the two extra tests from probe 1c) closes and lands; stage 1a follows. #96's observed-progress waits may stay after A.
5. Q5 = include (owner 2026-11-11): the 12 wall-bound tests of Kernel `tests/device_paced.rs` (6 are phase7 mutation filters G05, G08, G36, C05, C06, C07) are stage 3, classified by P3: the ms bounds and G05 become observations (#89 S1's park-probe shape) or second-scale "must end" bounds; the 1 s bounds that meet P3 stay.
6. Q6 = both, staged (owner 2026-11-11): 1c-i in stage 1 — the 15 `drop(frozen)` sites become the participant guard; 1c-ii after stage 3 — the ~75 `Frozen`/`released` uses, starting from a classification table: replace with the guard where it fits, keep `Frozen` where time must stay stopped while the test waits for an owner's reaction (the guard cannot do that: a non-clock wait under it hangs); remove `Frozen` only if nothing needs it.

### After Sol's design review (sol-design.md: FAIL, 4 majors) — owner 2026-11-11
- Q3 reopened: Sol majors 2–3 refute its premise (a fixed real-time delay does not guarantee device-time lateness, since the manual clock stops while any participant is busy; a single large delay misses non-monotonic windows; the scan misses time sources such as server replies' `now`, raw `RunHandle`s, `device.time_now()`). **Q3 = (c)**: move timing-sensitive scenarios out of Run-based rigs into Provider-direct rigs (closed by A); Run-based rigs keep only wiring tests that do not depend on a receipt window; rigs stop handing out raw handles; delay injection stays as a diagnostic, not a proof. To check: whether the hardware-rehearsal tests (`n47s5_*` etc.) can move.
- Q6 corrected (Sol major 1): the wait classification of the `drop(frozen)` sites comes before 1c-i; sites that wait on channels for device-time work cannot take the guard as is.
- Sol major 4 (mutation honesty: real-time bounds still yield false kills; `mutate.py` counts timeouts as kills) joins this work together with #91.

### After Sol's re-review of v2 (sol-v2.md: FAIL, 2 majors) — owner 2026-11-11
- Rehearsal exception to Q3(c) (Sol v2 major 2): **(i)** a hardware rehearsal's trial is INCONCLUSIVE (infra) only when its receipt itself is independently shown late (the booking reached the Provider already past its window, from the receipt record of Q10); lateness after receipt, missing records and wrong phase transitions stay failures (the Provider's responsibility). Such rehearsals are not mutation filters for lateness (rule M-R, Q14). → withdrawn 2026-11-13 (it conflicts with RM-15: a delay at dispatch after receipt is host lateness the spec permits; §5.3b).

### Owner decisions on v3.1 (2026-11-12)
- Q7 = reject (panic) a second registration of the same thread on the same clock, before allocating it; refcount only if a test ever needs it (nested helpers, two rigs on one clock — none today).
- Q8 = (a): a row whose only kill is a bound declares `"kill": "bound"`; declared rows are counted and kept few.
- Q9 = (a): `FakeDevice::new` on the real clock emits a marker (off by default, env-gated), read from the baseline output; report-only in stage M, enforced after stage 3 classifies the 66 sites.
- Q10 = add: a UHD Module Manifest section `receipts` (each Action's receipt root tick, written at `book_burst`'s entry) — the witness rule (i) needs. → withdrawn 2026-11-13 (2c does not change the Manifest; whether diagnosis needs it is decided separately).
- Q11 = (a): the trial code shared by hardware tests and their rehearsals moves to the wrapper too (forwarding only); compile checked at the gate, behaviour at the next bench.
- Q12 = not now: (b′) rejected; reconsider only if stage 0's R table finds many tests that cannot move to direct rigs yet fail on delays inside a Kernel call (observed so far: the two ur_25 tests of 2026-10-27, fixed in stage 3e).
- Q13 = stage M first (after #96), then 1a; M makes honest only timeout/build/signal/watchdog and session_with/UhdRig creation bounds — the rest after 2a.
- Q14 = adopt rule M-R: rows that kill lateness, a phase, or lateness before receipt use a direct rig or a unit test as their filter, never a Run rehearsal; the rows to retarget come from stage M's full run.
- Q15 = yes: on hardware, a case with zero conclusive trials FAILs as 'not exercised'. → not built in 2c (its premise is gone: (i) is withdrawn and hardware has no new source of lateness INCONCLUSIVE; orchestrator decision 3, 2026-11-13).
- Q16 = (b″) (owner's refinement): a fake rehearsal whose trial is timing-INCONCLUSIVE fails with a timing marker; gate.py and mutate.py, after the whole run, re-run exactly those tests one at a time (--test-threads=1, exact filter), at most 2 times; pass if a re-run is conclusive and passes; counts in gate.json; manual cargo test shows a hint to re-run serially. No in-test retry.
- Q17 = yes: a capture's (an Action the Kernel itself receives) receipt lateness is judged from the Kernel action log's LogEntry.time; a capture's lateness is judged from the Kernel action log's LogEntry.time (one-sided witness). → 2026-11-13: the Provider-addressed `receipts` clause is deleted.

### 段階 0 の表の後（2026-11-12，`51-tables.md` と Opus の再確認 `~/.cache/ezsdr-fixes/2026-11-12-n51s0/recheck.md`）
- Q17（owner 了承，2026-11-12）：Kernel の action log の `LogEntry.time` は `submit` の入口で打たれる（`pipeline.rs:939`）ので受け取りの下限であり，「遅い」は示せるが「間に合った」は示せない．Q17 はそのまま（安全側）．`submit` の中で遅れた capture は FAIL のまま残る．段階 2c で capture の最初のサンプルを証人に使えるかを調べて退けた（Sink は最初の適格サンプルを選ぶだけで受け取りの時刻を持たない：`ezsdr-sink-capture/src/lib.rs:208-245, 451-463`）．Q17 は Kernel の log の片側の証人のまま．
- Q16 の文言：「判定できた」は「流し直しで時刻の印が出なかった」の意味．わざと遅く受け取らせる case は印を case ごとに出さない（段階 M と 2c）．
- 表の食い違い 2〜7 は各段の実装の細部（2a：fixture 4b は target つきの Spec `stop`，`protocol.rs` の `ea_15` の入口．2c：`receipts` は作らない（§5.3b）．R2 の窓の端 `J(e)` も試験側の literal に．M：fixture 3 は実時間の fake に長い `apply_delay` をまず試す）．

### 段 1b の後（2026-11-12，記録 `reviews/n51-1b-dispositions.md`）
- 受け取りの時刻：駆動役を登録すると，`wait_until` は owner の poll の格子の上で終わり，試験のスレッドと uhd-control のどちらが先かで受け取りが T か T＋1 poll になった（`s14_…` の S3 が負荷の下で落ちた）．`wait_until` の最後に 1 µs の時計の待ちを足し，受け取りは T の後の最初の poll に決まる（上の契約 2）．S3 の値は 71 ms のまま．
- §1.3 の probe 1b の行と §7.1：#96 が預けた 14 本は，遅延の注入で `submit`（13 本）と `s14`（`s14_…` だけ）で落ちる．`CancellationPair::new` の後の注入では 0 本だった．#96 の記録の「15 本」は 14 本（15 本目の `s1_parity_payload_cut_separate_from_padding` は #96 が手順で直した）．
- §4.2 の field の順：Authority の drop の時に uhd-clock が時計の待ちの中にいる時だけ順が見える．`UhdRig` の field は `_driver`（最初），`_authority`（Authority と drop の probe を一つの `AuthorityGuard` に）の順．「Authority の後に登録する」は観測できない（行はない）．

### 段 1c の後（2026-11-12，`work/n51-1c`；記録は gate を閉じる commit の `reviews/n51-1c-*`）
- 待ちの規則は一つ：rig の，試験が選んだ時刻で終わる待ち（`settle`，`wait_until`，`Direct::start`）だけが 1 µs の時計の待ちで終わる（R1）。観測した状態で終わる待ち（`UhdRig::wait_for_events`，`Direct::until`）は 1 µs を足さない（状態は見えた時点で全部そろっている）。段 1b の「`settle(d)` は d ちょうど進む」は「d ＋ 1 µs」に変わった（1b の 3 つの assertion を直した）。受け取りは待ちの終わりの順ではなく，待ち行列が決める（R2：push の時刻より後の最初の poll）。`settle(999 µs)` が poll の時刻に着く場合も同じ。
- T4 の「説明のつかない競合」（96 回のうち 14 回，10 回）は時計の欠陥ではなく，その binary が mutant M12（`Direct::start` が末尾の 1 µs の待ちの無い `wait_until` を呼ぶ）を含んでいたため。probe の script は source を戻しただけで build し直さずに binary を写していた。`start` の待ちの assertion と M12 の行（N51C-12）が必須になり，probe の binary は毎回 build し直す。
- `Direct` の `armed_on`：`_driver` を最初の field，`AuthorityGuard`（Authority と drop の probe）を最後の field に置き，登録は `radio.arm()` の後（失敗しうる最後の手順の後）。

### 段 1d の後（2026-11-12，`work/n51-1d`；記録は gate を閉じる commit の `reviews/n51-1d-*`）
- `drop(frozen)` は 15 でなく 17 か所：G は 4（`drop(rig._frozen)` の `ur_22_f8_…` を落としていた），W2 は 3（`e2b.rs:88`，E2b が `e1::shutdown` を写した）。4 N / 4 G / 5 W1 / 3 W2 / 1 H。grep は `drop(_frozen)` と `drop(rig._frozen)` の綴りを含める。
- guard は `released`（#76 の形：owner の仕事を参加者に渡す）でなく，表の言う `drive()`：`test_support::handed_over(frozen, &clock)`。試験のスレッドが owner の仕事を自分で走らせるので，その thread を **Frozen を放す前に**登録する（逆順は，その間に時計が進める）。順序は helper の試験 2 本（N51D-01，-02）が一度だけ固定する。
- 判定が試験のスレッドの走る時点に依存していたのは G の `ur_28_…` だけ（遅延の注入で，release と `tx.pass()` の間が 3 ms で落ちる）。ほかの G と W2 は，注入した点と長さでは依存が見えず（owner の操作の内側の遅れは全部は試していない），guard は構造のためで行は無い（guard の有無で結果が変わらない：同値）。`ur_28_…` は release の後に 50 ms 眠り，時計が動かないことを assert する（N51D-03）。
- W2：眠る 150 ms の参加者と `join` を消した。`shutdown` の 100 ms の窓は時計の待ちなので，試験のスレッドが参加者なら窓は進む。
- 見送り：`tx.rs` の `-deadline` の helper を持つ 3 か所（`Frozen` が無い）は 4 で扱う。

### 段 2a の後（2026-11-13，`work/n51-2a`；記録は gate を閉じる commit の `reviews/n51-2a-*`）
- 包みは `tests/common/run.rs` の `Run`（`acceptance` の `uhd.rs` は `#[path]` で同じ file を取り込む）。raw の生成と `RunHandle` は `run.rs` の外から消え，走査が止める。84 か所の型の付け替えと，生成 12 か所の `Run::connect`／`Run::start_spec` への切り替えは 1 つの compile 単位（sub-step 2）。検査の規則と保証（Run を終わらせ得る呼び出しの戻り，`finish`，`Drop`。read-only の戻りは検査しない）は §6.2，宣言は §6.2 の `Expect`（`Refused`／`Policy`／`Any`）と `join_is_subject`，`rx_join_issue_67`（#67）。
- 宣言の数：Policy の終わり 11 本（`DEVICE_LOST`，`ur_27_…` だけ `radio.CLOCK_LOST`），refuse の生成（`spec_run_ended`／`start_spec` と，`uhd.rs` の 3 本）。join が主題の 3 本，#67 の 5 本。`Expect::Any` は `perf_rx_rate_trial`，`b8_lead_trial`（どちらも `hardware.rs` から `common/mod.rs` に移し，FakeDevice の Lost と bound の試験を付けた），`run_stage`（観測だけの stage が Policy の終わりを記録するので，`Refused` でなく `Any`。orchestrator の判断）。B9 の 3 本は `Policy(DEVICE_LOST)`。J317，J318，P880，R15（phase 7）は `"kill": "bound"`（N51-2A-21 は同じ mutant で submit が待つ形の覆い）。規則は変えず，実機の期待値も変えていない。
- 既定の suite の試験が上限をわざと起こすときは `capture_markers(&["join"], …)` で，期待する上限の marker だけを取り込む（ほかの上限の marker は普通に書かれ，上限を期待しない試験は何も取り込まない）：output に `EZSDR-INFRA:` の行が残ると，runner は上限が切れたと読む。`Drop` が unwind の中で live の Run を `finish()` して marker だけ書く形は，M3 の列（`events()` の後の assert の失敗）を閉じる。
- fixture 4・4b・6・7・8（`Stalls`，handshake）と `mutate.py --e2e` の 14 case（9 → 14，`lacks`）は §6.4。`protocol.rs` の `handle` 82 か所，走査の形（24 行の許可表，3 fn の `Any`）と診断の注入（`rig_delay`）は §5.4。走査の許可表に，Run を動かす時刻読みの 3 本（`ur_29_a_silent_stream_is_a_lost_device`，`ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one`，`ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx`）が理由つきで載る（実時間を使うこと自体が目的か，実時間の予行が要る。直結の rig への移しは 2b／2c）。mutation の行は `N51-2A-01`〜`39`（全 list：2 365 行のうち通らないのは既知の `phase7/G09` だけ）。
- 診断の sweep（`{now, advance, submit}` × {5, 20, 60, 200} ms，`fake` 463 本，`uhd` 31 本，`protocol` 79 本，各 1 回）：注入で verdict が変わるのは `fake` の 13 本と `uhd.rs` の 1 本（`ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end`）で，`protocol.rs` は 0 本。落ちるのは §5.2 の表の集合（`n47s5_*` の予行，`ur_21`／`ur_23`／`ur_29` の R1／R2）に，`now` で 2 本（`p70_…`，`s1_…`）が加わったもの。全部が自分の assertion で落ち，marker で落ちたものは無い：包みは Run の終わり方を判定するのであって，試験のスレッドがいつ動くかは見ないので，この 14 本は 2a の検出の外で，段 2b／2c の入力である（§5.2 の R 表）。
- 流していない・限界：`hardware` binary は libuhd の無い環境で build していない（gate の `hw-build`）。B9／B8／perf の宣言は compile 検査だけ。`Drop` の `catch_unwind` と，`rig_delay` の呼び出し（環境変数を 1 回だけ読む）は試験が無い。`protocol.rs` の注入が `handle` の戻りの**後**であることは時間でしか見えないので固定していない。

### 段 2b の後（2026-11-13，`work/n51-2b`；記録は gate を閉じる commit の `reviews/n51-2b-*`）
- 試験側だけの変更（production code は変えない）。A：走査に `assemble_on` の規則（`ezsdr-acceptance` の試験 file のうち `uhd.rs` 以外に `assemble_on` が出たら違反：`support::session_run_with` の raw `connect` に FakeDevice の assembly を渡せないようにする），`ur_29_a_silent_stream_is_a_lost_device` と `ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx` の時刻読み（device 時計と実時間の assert）を消して許可表の 2 行（24 → 22）を外した，`bounded()` と `ur_21_a_descheduled…` の `entered.recv_timeout` が切れたら `EZSDR-INFRA:` を書く（表 §4.3 のとおり。行の kill が hang だけのものは `"kill": "bound"` か付け替え）。B：`ur_21_s18…`（最初の repeat を untimed），`ur_23_a_burst_after_a_stop…`（`start_repeat`。`rejections().is_empty()` は N47S4-41 の唯一の killer なので残す），`ur_29_a_loss_found…`（untimed の `send`），`ea_16_untimed_repeat_after_stop…`（Stop 自身の log の時刻を使う）。C：`ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` を Run の半分（id の対応と corrected 値。untimed，60 ms の待ち）と `Direct` の窓の試験 `ur_21_a_refused_configure_keeps_its_burst_s_span_until_its_booked_end_on_the_direct_rig`（行 N51-2B-06）に分けた。
- 2c へ送ったもの 13 本：`n47s5_*` の予行 7 本，`s1_short_first_sends…`，`p70_one_sample_burst_then_timed_burst…`，`rx_start_lead_group_runs_on_the_fake`（R2。`restart_trial` の `start_rx` が値の `set` の apply の窓の中で受け取られる必要がある），`rehearsal_b7_*` 3 本。どれも共有の trial で，直結に移すと予行でなくなり，期待は受け取りの時刻を要る（多くは Provider の `receipts`。`n47s5_stop_end_rehearsal_on_the_fake` は Kernel の log で，2a の sweep では落ちなかった。sweep で落ちたのは 13 本のうち 9 本）。`ur_21_a_descheduled…` は 3 段へ。
- sweep（2a の注入 12 点）を moved の 6 本（`ur_21_a_refused_configure_refuses_its_burst…`，`ur_21_s18…`，`ur_23_a_burst_after_a_stop…`，`ur_29_a_loss_found…`，`ur_29_a_silent…`，`ur_33_a_shutdown…`）と `ea_16_…` に流して全部通る。`ea_16` の下限（`gap ≥ 9 900`）は Stop の log の時刻（Kernel が compile と dispatch の前に打つ，`pipeline.rs:939`）を基準にするので，その後でスレッドが止まれば Provider の受け取りとの差は上限なく開き，実際の受け取りからの間隔は示さない。正確な窓は 2c の `receipts` の義務として残す。

### 段 2c の後（2026-11-13，`work/n51-2c`；記録は gate を閉じる commit の `reviews/n51-2c-*`）
- 試験側と tooling だけの変更（production code と Manifest は変えない。`src/` は `control.rs` の `#[cfg(test)]` だけ）。A：`mutate.py` の baseline を `judge` に通す。B：`iii::late_evidence`（§5.3b の証拠 2 つ），`iii::kernel_refused`，`Rehearsal`（admit 違反を証拠より先に plain な失敗に，印は `finish` で 1 回），`governance.rs` の走査（予行 9 本（fix 1 で N2 の late の 1 本が足され 10 本）の `.trial(` と `finish()`，span の外の assert 系の macro と `_verdict(`／`_outcome(` の呼びを表と照合），実機と共有する判定の `kernel_refused` 先行，long close の形の欠けを FAIL に。C1：共有 trial が `Lateness` を持つ，held-tail の C = 1 000（F2），`stop_next_trial` の N2（実機は N2 の行を 1 本足す：`stop_after_margin_verdict`，N1 の stamp が margin を過ぎていれば INCONCLUSIVE，N2 自身の late の drop も INCONCLUSIVE（fix 1：`late_arrival_verdict_of("N2", …)`））。C2：予行 9 本を report-only から差し控えに切り替え（`report_only` は消えた），README を書き直した。
- 単独の 20 回（9 本，180 回）は印 0，plain な失敗 0。8 並列 3 回（各 24 回）は front の 24 回中 4 回が印（すべて `T2 2 samples at 10000000 S/s` の `TIME_ERROR` cause late）で，plain な失敗は 0。印の回は単独で流し直す（§5.3b）。
- 診断の sweep（2a の注入 12 点を，予行 9 本と `rx_start_lead_group_runs_on_the_fake` と `rehearsal_b7_*` 3 本の 13 本に流す。注入なしは 13 本とも通る）：予行が落ちる config は 12 点すべてで，落ちた回の**印つき**は 点ごとに 1〜8 本（`now` 5・6・8・8，`advance` 1・2・2・2，`submit` 1・2・2・5），plain な失敗は次の 2 種だけ。(a) `rx_start_lead_group_runs_on_the_fake`（`submit` 20・60・200）は 2a のときと同じ（段 3 へ送ったもの：§3.4）。(b)（C3 で直した：下の C3 の行）`n47s5_held_tail_and_stop_rehearsal_on_the_fake` は `now` 60・200 と `submit` 60・200 の 4 点で plain に落ちる：わざと遅らせる case N7（印を出さない，共有の判定のまま）の 1 つ目で，**C 以外の Action（A と capture）も host の遅れで drop される**（`TIME_ERROR` が 2 本：A の target と C の target。`now` 60 で A の stamp が deadline を過ぎているのを確かめた。ほかの 3 点は同じ形の INFERRED）ので `late_arrival_verdict` の guard（`time_errors == C の late の drop の数`）に入らず FAIL になる。2c の前から同じ形（2a の sweep の同じ test）で，N7 を述語の外に置く決定（§2.4）の帰結である。直し方は orchestrator の判断（N7 の 2 case も C の late の drop を除いた `Lateness` で `Rehearsal::trial` に通す案が最小）。
- C3（N7 を述語に通す）：上の (b) の直し。N7 の 2 case を `iii::without_expected_late`（C の late の drop と deadline だけを除く）の `Lateness` で `Rehearsal::trial` に通した。guard の表は held-tail が sites 4→6，`direct` 2→0，`outside` 2→0，`calls` 2→0。再実行：`now` 60・200，`submit` 60・200 の held-tail は印だけで plain な失敗 0（6 trial が印，N7 の 2 case は A の遅れで判定されず），注入なしの単独 20 回は印 0・失敗 0 で N7 の 2 case は判定され INCONCLUSIVE（host latency）。
- Sol のコード review の直し（fix 1）：(M1) `one_sample_pair` の test（`one_sample_pair_returns_the_manifest_its_time_errors_were_read_from`）が `Rehearsal` を通らず「burst が 2 本」を assert していたので，注入した遅れで plain に落ちた（`now` 60：0 本，`submit` 60：1 本）。A を 1 ms 先（deadline を過ぎる）に booking して A の late の drop を作りだし，Manifest の読みの同一性（`time_errors == time_error_pairs(manifest)`）と drop が 1 本以上あることだけを assert する（どの遅れでも成り立つ）。(M2) 実機の Session の Stop で，N1 は margin の中，N2 だけが host の遅れで late の drop になると，どちらの判定行も run 全体の `TIME_ERROR` を plain な FAIL にしていた。N2 の判定は自分の late の drop を `iii::late_arrival_verdict_of("N2", …)`（INCONCLUSIVE，host latency）に通し，N1 の判定は N2 の late の drop の数だけ `TIME_ERROR` を引く（`iii::without_late_drops`）。ほかの fault（別の TIME_ERROR，underflow，device の fault）は今までどおり FAIL。`stop_next_trial` は `n2_lag`（実機は `None`）で N2 を deadline の後に出せる：fake の `n47s5_stop_n2_late_drop_is_host_latency_on_both_lines`（N2 の deadline と late の drop だけを除いた `Lateness` で `Rehearsal::trial`）が両方の行を，`n47s5_stop_n2_late_case_leaves_out_only_n2s_own_late_drop`（Rehearsal を通さず件数だけ）が除外を決める。Provider は N2 の stamp が自分の `T − 3 ms` 以降のとき late で drop した（`n2_lag` を −3.0〜+1.0 ms で測った：−1.5 ms までは drop 無し，−1.0 ms 以降は drop）：1.5 ms の余裕で `n2_lag = 0.5 ms` を使う（fake の manual clock，INFERRED：実機では同じ値を測っていない）。(minor) `late_arrivals` の本体を `late_arrivals_in`（payload の列）に分け，A と C の drop が別の target にある合成の payload で target の選び方を決める（N47S5-192 の killer）。
- 流していない・限界：実機の bench（N2 の行，N2 の late の drop の行（fix 1 で足した。fake の `n47s5_stop_n2_late_drop_is_host_latency_on_both_lines` だけで，実機では流していない），C = 1 000 の onset，long close の FAIL）は次の bench。`rx_start_lead_group_runs_on_the_fake` は段 3。
