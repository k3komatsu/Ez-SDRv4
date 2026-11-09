# 設計メモ 49：時刻と予約のモデルの棚卸し

Status: **決定済みの設計メモ**（owner 2026-11-08．§11 はすべて決定．2026-11-09 の実機の結果を §8 に入れた：§3 などに残る「pending 2026-11-09 (x)」は §8 の表で読む）．第 3 版＋再確認の直し（2026-11-08．設計の段〈AGENTS §9 手順 0〉は narrow な再確認 PASS WITH MINORS で終わった．minor は §12.1．Opus subagent，読むだけ，tree の外）．**§11 はすべて決定済み**（owner 2026-11-08）：§11.1 は D′（初めに D を選び，Sol の方向レビュー〈SOUND WITH CHANGES，`~/.cache/ezsdr-fixes/2026-11-08-n49/sol-direction.md`〉の後に D′ へ改めた），§11.2〜§11.10 は各節の「決定」の行．第 2 版は Sol の finding 9 件と事実の訂正 2 件を入れた．第 3 版は第 2 版への Opus の再確認（FAIL，方向と決定はそのまま，`~/.cache/ezsdr-fixes/2026-11-08-n49/recheck-rev2.md`）の M1〜M5，m1〜m4 と，決定とのずれの残りを入れた（Changes）．AGENTS.md §9 の手順 0．根拠は HEAD `134b273`．2026-11-09 の実機の結果はまだない（`pending 2026-11-09 (x)` と書いた所）．第 1 版は `~/.cache/ezsdr-fixes/2026-11-08-n49/49-time-and-booking-inventory.rev1.md`．

| 項目 | 内容 |
|---|---|
| きっかけ | note 47 段階 5 の Sol S1〜S4 と，その直しへの Opus の再確認の FAIL 3 回は，どれも受付で決める予約の記録（`ezsdr_radio::staging::Bookings`）と UHD の送信の持ち主（uhd-tx）の実際の寿命のずれだった（`plan/phase8/reviews/47s5-dispositions.md:132-134`）．機構 `planned-state-read-as-actual` は issue 13 件で，3 回をとうに超えている（`plan/maintenance/failure-mechanisms.md:9,53`）．owner は段階の後の棚卸しを命じた（`47s5-dispositions.md:60`） |
| 決めること | 時刻と予約の規則を一つずつ，残すか，簡単にするか，消すか．同じ intent の写しを一つにする形（決定：D′）．機能を削る候補（owner が一つずつ決めた，§10，§11） |
| 変えないもの | brief の原則 1〜6（時刻なしは Kernel の性質，規則は一枚の表，拒否は利用者の約束と照らす，RM-15 の原則，設計メモ 46 の F1〜F9，AGENTS §3）．Kernel，Stream Contract，Vision §58・§61 |
| 範囲 | 送信の時刻と予約：Kernel の `at`・late policy・時刻なし，記録，UHD の持ち主，MockRadio．受信は範囲の外（ただし同じ lock を使うので §5.4.9 で触れる） |
| 印 | VERIFIED（コードか記録を読んだ），INFERRED（推論），pending 2026-11-09 (x)（`plan/phase8/bench.md` の 2026-11-09 の手順の項目 (x)） |

## 1. 要約

**決定（§11.1）：D′「一つの状態機械と，一つずつの操作」．** 送信 stream ごとに，予約・装置の burst の区切り・Stop・差し替え，**そして実行の結果**を，`ezsdr_radio` の純粋な一つの状態機械が持つ．uhd-tx は実行役で，状態機械から**不変の操作（token）を一つずつ**受け取り，装置を呼び，結果を返す．

- 消えるもの：同じ intent の独立した写し（記録と Own の `held`・`open`・`playing`・front の予測，受付から Own への mpsc の配送の窓）．
- 消えないもの：予定と実際の I/O の違い（short send，例外，configure の拒否，underflow）．D′ はこれを隠さず，一つの持ち主の中の実行の状態（封じた／発行した／送った／鳴った）として表す（§5.4）．
- D から残したもの：controller のスレッドを足さない，新しい地平の値を足さない（地平は W），uhd-tx が自分の turn で引く．
- D から改めたもの：「確定したものは報告で変えない」をやめた．configure と recovery は payload の発行の前提にした．時刻による受付の封じ，操作の発行，実行の結果を別の概念にした．最初の移行では今の振る舞いを変えない（Stop は実際に発行した所で閉じる）．

**段階（§12）：** 振る舞いを変えない抽出を先に，契約の変更を後にする．抽出は四つ．(1) 共有の実装を呼ばない spec の oracle を今のコードで固める．(2) 予約と Stop の置き場所を，受付の順の一つの列にする．(3) 操作と結果の protocol を入れる（最も risky）．(4) 区切りの純粋な判断を両 Provider で共有する．契約の変更（KL-38 の撤回，差し替えの結果の正直な記録など）は，両 Provider を同時に変える．

**§11 の決定（owner 2026-11-08，すべて決まった）：** X4 は退ける（Sol の反対を採る：TDD の slot を先に予約した後に，その前へ反応の ACK を入れる使い方）．KL-38 は X2 ではなく C1「configure を payload の発行の前に終え，発行していない piece を区切り直す」で消す．short head と Stop の最初の移行は (b)（今のまま）．X1 は採る．X3 は採らない（11-04 の決定を残し，送信の rate の変更は後の独立した段階）．原則 4 の当てはめは (a) footprint．継ぎと遅い継ぎは残す．差し替えの outcome は C2 で正直にする．

## 2. 利用者に見える約束（今と D′ の後）

「最初の移行」は §12 の N0 と抽出（O1，E1〜E3）の後，「契約の変更」は §12 の C1・C2 の後．

| 操作 | 今 | 最初の移行の後 | 契約の変更の後（§11 の決定） |
|---|---|---|---|
| `tx.repeat(x, at)`（時刻つき） | `at` から．遅れたら SendAsap で最初の空きへ動き `TIME_ERROR{send_asap}`．流れている repeat があれば `Rejected`（EA-16 `design/16-easy-api.md:85`，RM-12，RM-14） | 同じ | 同じ |
| `tx.repeat(x)`（時刻なし．`python/ezsdr/session.py:491` は `at` を `None` で送る） | max(at，普通の開始) 以後の最初の空き．Provider は遅れと判じない（RM-15，RS-19） | 同じ | 同じ |
| `tx.switch(x)` | 流れている repeat の周期の境目で．境目は「受付＋W＋device lead」以後の最初．保留は一つ．Stop・loss・`Provider::stop`・打ち切りで取り消し．host が遅れたら次の境目へ動き `TIME_ERROR`（RM-37，UR-38） | 同じ．取り込みは受付の時に決まるので，Own が取り込む前に Stop が来た時だけ UHD が `cancelled` にする今の時刻しだいの差は，Mock の今の答え（境目が B より前なら `applied`）にそろう．host の遅れで境目を動かす道（UR-38 の S12）も届かなくなり，`moved_to` は両 Provider で `null` になる（INFERRED，§5.4.5） | `switches` の行の outcome が，境目の sample を実際に送ったかで決まる（C2，§11.10）：Stop が境目の前で閉じれば `cancelled`，境目を運ぶ送りが例外で終われば `uncertain` |
| `tx.stop()` | 渡した sample は出る（W = 10 ms 以内）．次の repeat は，時刻なしなら Stop の範囲の後，時刻つきで範囲に当たれば動いて `TIME_ERROR`（EA-16，RM-16） | 同じ（実際に発行した所で閉じ，保持した最後の sample も閉じの send で出る） | 同じ．規則の cursor まで送る (a) は今は採らない（§11.8 の決定） |
| raw の `send`（`Session.submit` か Spec の schedule．Easy API にはない） | 時刻つきは DropAndFlag．受け付けた予約に重なれば受付で `Rejected`（O2） | 同じ | 同じ |
| 前の burst の終わりちょうどの `send`（継ぎ） | 前が 32 sample を超え，J(e) = e − 5 ms までに届けば一つの device burst（RM-15，UR-23）．前が 32 以下なら詰めた範囲の中は拒否（KL-36） | 同じ | 同じ（§11.7 の (a) は別の変更として後で決める） |
| 受け付けた burst より前に始まる `send` | その burst が W 以内に始まるなら拒否（KL-37）．それより先なら受ける | 同じ．「渡したかもしれない」が「封じた」という時刻の定義になる | 同じ（X4 は退ける，§11.6） |
| 送信の値（周波数・gain・antenna） | stream の予約がすべて終わった後だけ（RM-21，KL-35）．configure を拒まれると，後ろに継いだ短い piece は閉じ，その後ろは遅れる（KL-38） | 同じ | configure を拒まれた burst だけが拒否され，その後ろの継いだ piece は区切り直されて時刻どおりに鳴る（C1，§11.4．KL-38 の撤回） |
| 送信の rate | bench の確認待ちで拒否（EA-16） | 同じ | 同じ．11-04 の決定（空いた stream の予約の parameter）は D′ の後の独立した段階で作る（§11.5） |
| host の遅れ，装置の故障 | 非同期の `TIME_ERROR`・`TX_UNDERFLOW`．打ち切った repeat は終わり，保留の差し替えは取り消し（UR-22，RM-37） | 同じ | 同じ（差し替えの outcome だけ上の C2） |

今も後もできないこと：封じた（W 以内の）sample の予定を受付で変えること．発行した操作を取り消すこと．受け付けた予約に重ねること．流れている間に値を変えること．

**Easy API と raw の差（VERIFIED）．** Easy API の送信は `repeat`・`switch`・`stop` だけで，有限の burst（`send`）はない（`python/ezsdr/session.py:459-512`．`grep -n "def send" python/ezsdr/*.py` は何も返さない．`grep -rn '"verb": "send"' python/` も何も返さない）．repeat は終わりがないので，Easy API からは継ぎ・順番の逆の予約・KL-36・KL-37・KL-38 の場面に届かない．これらに届くのは raw の `Session.submit`，Spec の schedule，Phase 10 の Processor・Reactor の burst である（Vision §22：「A Processor or Reactor must be able to generate a burst dynamically during RUN」，「packet responses, ARQ, TDD, reactive protocols」，`design/vision/04-execution-model-and-data-contracts.md:223-233`）．Phase 10 の使い方は raw と同じ道を通るので，「raw だけだから損はない」とは言えない（§11.6）．

## 3. 規則の表

略号：**記録** = `crates/ezsdr-radio/src/staging.rs` の `Bookings`．**Ctl** = uhd-control（`crates/ezsdr-radio-uhd/src/provider/control.rs`）．**Own** = uhd-tx（`crates/ezsdr-radio-uhd/src/provider/tx.rs`）．**Mock** = `crates/ezsdr-mock-radio/src/lib.rs`．**行** = `plan/maintenance/tools/mutations.json` の mutation 行．**写し** = 同じ事実を持つ所の数．bench-results は `plan/phase8/bench-results.md`．「D′ で」の列の段階の名前は §12．

| # | 規則（どこ） | 支える約束 | 実機 | ないと壊れるもの（列 → 見える誤り） | 簡単な代案と D′ での形 | 写しと固定（テスト・行） |
|---|---|---|---|---|---|---|
| K1 | `at` を送信の SampleClock の格子へ．不正確なら次の sample，要求は残す（SC-23a，`crates/ezsdr-kernel/src/stream/burst.rs:31-43`） | RM-14 の `requested_target` | —（算術） | 格子の間の開始．記録に要求が残らない | なし | Kernel だけ．spec 02 の試験 |
| K2 | 時刻なし：`at` のない Vocabulary action は earliest（今＋Provider の lead，clock の origin 以後）で admit し，`Dispatched.untimed` を付け，要求の時刻を持たない（RS-19 `design/04-run-and-session.md:180-181`，`coordinator/pipeline.rs:1143-1167`，`session.rs:362-377`，`coordinator/admission.rs:161`，`module_api.rs:921`） | 原則 1，EA-16 の `at` なし | pending (f) | 時刻なしの repeat が遅れとして `TIME_ERROR` を受ける（F36 の前の形） | なし（決定済み） | Kernel だけ．`rs_19_*`，`kc_28_*`．N47S5 の Kernel の行 6 本 |
| K3 | `LatePolicy::decide`：target − now ≥ min_lead なら OnTime，他は SendAsap・Drop・PlanViolation（SC-27，`burst.rs:104-123`）．verb で固定：`start_repeat` は SendAsapAndFlag，`send` は DropAndFlag（RM-12 `design/07-radio-model.md:142`） | RM-14，Vision §22 | Module を通る最小の lead は 3 ms（2 ms は 0.44〜0.51 ms 遅れて drop），#1・#2（bench-results:49，VERIFIED）．profile の 5 ms は INFERRED（KL-15） | 遅れた burst の扱いが決まらない | なし | Kernel だけ．`sc_27_*` |
| R1 | 判定の瞬間と普通の開始 `ordinary_k`：max(今，origin − lead) の次の sample＋device lead（RM-14 `design/07-radio-model.md:157`，`staging.rs:597-599`，`timeline.rs:334-336`） | RM-14 | device lead 2 ms は INFERRED（`design/18-uhd-radio.md` §3） | Mock と Module が 1 sample 違う（計画の再確認 m1） | なし | 共有 1 本．呼び手は Ctl `control.rs:503`，Mock `lib.rs:1633`，Own の dispatch で再計算（`tx.rs:1043-1047`）．行 N47S5-86 |
| R2 | 重なりの拒否：受付の順で後に来た方を拒む．向きによらない（RM-15 `design/07-radio-model.md:161`，`staging.rs:226-247,537-554`，UR-21，MR-16） | 原則 4，O2（submit の返事で `Rejected`），EA-16「二つ目の repeat は Rejected」 | —（約束） | Mock は動かし Module は dispatch で遅れにする（#79）．利用者には非同期の `TIME_ERROR` だけ | X4 で比較一つになるが，X4 は退ける（§11.6）．D′ では状態機械の受付の遷移 | 共有 1 本．`rm_15_*`（`crates/ezsdr-radio/tests/staging.rs`），`ur_21_s1_*`，`s15_parity_booking_rule_both_receipt_orders`．行 N47S5-05/07/08/18/19 |
| R3 | front：単独の頭は 1〜3 sample なら start＋5，4〜32 なら end＋1．32 を超える頭と継いだ piece は end＋2．repeat は Stop まで（RM-15，`staging.rs:586-592`） | RM-15，KL-36 | +2：end＋1 は late，+2 は 0（2026-11-06，二台，bench-results:369，VERIFIED）．閉じる最初の send は 4 以上（2026-11-05 C++，1・2 MS/s，:322-332，VERIFIED）．詰めた後の +1 と，4〜32 の end＋1 は FakeDevice だけ（`crates/ezsdr-radio-uhd/src/device.rs:689`）で INFERRED．pending (c)1，(c)2 | 前の burst の閉じ方の後ろの sample に置いた予約を装置が捨てる（別の burst の gap 0 の SOB，bench-results:100,128，VERIFIED） | D′：front は状態機械の区切り（E3 の純粋な判断）が出す．受付はその同じ関数で決める | **二つ**：記録の `front()` が予測し，Own の `send`・`close_device_burst`・`advance_front`（`tx.rs:614-805,849-887,177-180`）が実際を作る．ずれを数える stat `tx_close_past_record_front`（`tx.rs:551-553`）がある．Mock は記録を使う．行 N47S5-02/03/04 |
| R4 | 継ぎ：32 sample を超える chain の終わりちょうどの予約は，J(e) までに届けば一つの device burst として続く（RM-15，UR-23 `design/18-uhd-radio.md:257`，`staging.rs:226-247,558-560`） | F7 (b)（owner 2026-11-04，`plan/phase8/46-streaming-feature-ledger.md:226`），Vision §22（TDD） | 一つの device burst に続ける：`hw_b8_burst_at_a_sent_burst_s_end` PASS（2026-11-06，二台，bench-results:342,369，VERIFIED）．別の device burst の gap 0 は落ちる（VERIFIED）．保持の尾での継ぎは pending (d)1 | gap 0 の予約が装置に捨てられ，burst が丸ごと失われる | X6（継ぎを消す）は退ける．D′ では「継ぐか閉じるか」は J(e) で封じる時刻の述語（§5.4.10） | **二つ**：記録の `join`・`joined`（`staging.rs:85-91`）と，Own の `TxChain::Open{next, tail}`＋`held`（`tx.rs:104-112,220`）．行 N47S5-05/06/14/54/57/58/133/142 |
| R5 | J(e) = instant(e) − (device lead＋delivery allowance) = e − 5 ms（RM-15，`core.rs:924-926`） | 継ぎの締め切りが，普通の時刻つきの開始の lead と同じ 5 ms（KL-13 の回避策「5 ms 前に」） | lead と allowance は INFERRED（spec 18 §3）．pending (d)2，(d)3 | 締め切りが Own の close と競う（計画の再確認 M1(c)，`47s5-opus-rechecks.md:144-151`） | X5 は退ける．D′ では受付の now を lock の中で読み，競争そのものがなくなる（§5.4.10） | 共有 1 本（`Core::join_by`）．Mock は `lib.rs:1636`．`ur_21_m1_receipt_at_the_deadline_is_decided_by_the_record`．行 N47S5-54/58 |
| R6 | KL-36：32 sample 以下の burst は一回の send で閉じ，その詰めた範囲の中の予約は拒む（RM-15，UR-22 `design/18-uhd-radio.md:253`，KL-36 `KNOWN_LIMITATIONS.md:134-139`） | owner の D1（2026-11-06，`46-streaming-feature-ledger.md:228`），#79（2026-11-05，`failure-mechanisms.md:10`） | 開いたままの最初の send が 28 sample 以下で続きが後から来ると，補間 80 以上で Underflow．30 以上は起きない（2026-11-06，bench-results:345-353，VERIFIED）．32 はその上の安全側の値（`device.rs:657-663`） | 開いた短い最初の send → Underflow → 続きが黙って失われる（`47s5-opus-rechecks.md:251`） | 最初の移行は今のまま（§11.7 (b)）．短い頭への継ぎは後で別の変更として決める | 記録の `len > 32`（`staging.rs:244,589`）と Own の `short_first`（`tx.rs:642`）．行 N48S3-07/08，N47S5-06 |
| R7 | KL-37：渡したかもしれない chain より前に始まる予約は拒む．chain の開始 < at_or_after(今＋W) で，その前に開いた継ぎがないとき（RM-15，`staging.rs:565-567,615-617`，MR-17） | owner の「B-first」（2026-11-07，`47s5-dispositions.md:57`） | Own は at_or_after(now＋W) より前だけを渡す（`tx.rs:454-455`，コード）．渡した sample は取り消せない（Stop の終わり ≈ W，VERIFIED） | 後の chain の sample が装置にあるのに，前の burst を送ることになり順番が崩れる | X4 は退ける．D′ では「封じた」（時刻の述語）が定義になり，予測でなくなる | 記録が Own の振る舞いを**予測**する（`may_be_handed`）．`ur_21_s5_b_handed_before_an_earlier_a_is_refused_at_receipt`，`s15_parity_b_first_accepted_booking_rule`．行 N47S5-81〜84/105/106 |
| R8 | Stop：境 B = at_or_after(t＋W)＋3．残る chain の front を B で切り，継ぎを止める．B より前に始まる予約を取り消した時だけ範囲 [t, B) を足す（RM-16 `design/07-radio-model.md:167-168`，`staging.rs:346-368,604-611`，`core.rs:399-404`） | RM-16「渡していない held の burst は取り消し，その後の burst は普通に送る」，EA-16 の `stop()` | Stop の提出から音が止むまで 9.3〜10.5 ms（今の実装の三つの記録，bench-results:229-245．2026-10-20 も :50，VERIFIED）．+3 は INFERRED．pending (d)4〜(d)6，(g) | Stop の直後の予約が，まだ鳴る渡した sample に重なる（装置の `TIME_ERROR`．Mock は鳴った sample に重ねる） | D′（最初の移行）：実際に発行した所で閉じ，保持した尾は閉じの send で出る．受付の範囲は今の B のまま．Mock の M2 は E3 で MR-25 の式のまま共有の区切りで計算する（Stop の sample が下限，§5.4.8） | **三つ**：記録の `halt` と範囲，Own の `stop_now`→`end_open`（handed cursor で閉じ，held を取り消す，`tx.rs:1149-1156,823-847`），Mock の `tx_handed_end`（`lib.rs:1393-1420`）．行 N47S5-09/10/16/17/23/24/29/30/55/62〜65/67/69 |
| R9 | Stop の範囲だけに当たる時刻つきの SendAsap は範囲の後へ動き `TIME_ERROR{send_asap}`．DropAndFlag は拒む．時刻なしは後ろへ置く（RM-16，F36，`staging.rs:304-311`，Ctl `control.rs:525-541`，Mock `lib.rs:1662-1677`） | EA-16「`stop()` の後の repeat は送る」，原則 4 | pending (f)1 | `repeat; stop; repeat` が拒まれる（F36，`47s5-selfreview.md:50`） | なし | 共有 1 本，呼び手 2 か所．`rm_16_a_timed_send_asap_moves_past_only_the_stop_region`，`ur_21_s16_*`，`mr_16_timed_send_asap_in_the_stop_region_moves_with_time_error`．行 N47S5-115/117/118/126/127 |
| R10 | 時刻なしの置き場所：max(at，普通) 以後で，どの範囲・Stop の範囲・R7 の chain にも当たらない最初の sample．Provider は遅れと判じない．dispatch で遅れたら黙って動かす（RM-15，`staging.rs:281-299,317-319`） | 原則 1，EA-16 | pending (f)2 | 時刻なしの burst が予約に重なって拒まれるか，遅れとして報告される | X4 は退ける（隙間への置き場所を残す）．D′ では発行の時の移動も状態機械 | 共有，加えて Own の `dispatch_start` の時刻なしの枝（`tx.rs:1059-1077`）．行 N47S5-112〜114/116/119〜125/129/130 |
| R11 | 受付で遅れた SendAsap の置き場所：普通の開始以後の最初の空き（RM-14，`place(ordinary)`） | RM-14 | — | 遅れた repeat が別の予約に重なる | なし | 共有．加えて Own の dispatch の移動（U4）．行 N47S5-20/21 |
| R12 | prune：front が受付の sample 以前の chain と，過ぎた継ぎ点を落とす（`staging.rs:327-338`） | lock の下の仕事を生きている chain の数に抑える（§5.4.9） | — | 長い Run で記録が伸び，lock の下が遅くなる | D′ では，発行した操作は prune の外（結果を受けるまで残す） | 共有．F10（prune した頭を止まっていた Own が遅れて動かし，追われない burst ができた）はこれと Own の写しのずれ．行 N47S5-87/142 |
| R13 | 値の busy end：最後に予約した burst の終わりが受付以前の時だけ送信の値を受ける．Stop は B で切る（RM-21 `design/07-radio-model.md:233`，`staging.rs:190-217`） | F2 ①，KL-35 | T5：前の burst の ACK の後の時刻なしの設定は末尾を乱さず，e の 4.6 ms 前の設定は乱す（2026-11-05，二台，bench-results:311，VERIFIED） | 流れている burst の末尾が新しい周波数になる | なし | 共有 1 本（`until`）．ずれの finding なし．行 N47S4-01〜04/17/17b/28/30/31，N47S5-01 |
| R14 | 差し替え：流れている repeat の格子で，max(at，受付＋W＋lead) 以後の最初の境目．保留は一つ．Own が cursor で取り込み，過ぎていれば SendAsap は次の境目へ・Drop は拒む．境目で適用．Stop（B ≤ b）・loss・`Provider::stop`・打ち切り・F8・dispatch の drop で取り消し（RM-37 `design/07-radio-model.md:346`，UR-38 `design/18-uhd-radio.md:297`，MR-42，`staging.rs:375-450`） | F7 の差し替え（owner），EA-16 の `switch` | pending (e) | 周期の途中で切り替わる，または渡した sample と競う | D′：境目は操作の始まりで切る．最初の移行（E2a）は受付の時に発行した cursor で取り込み，outcome は今の規則．C2 で，境目を運ぶ送りの結果と装置の時刻で `applied` を決める（§5.4.5，§11.10）．X1 で Drop の枝が消える | **三つ**：記録の `pending`（installed，moved_to）と `settled`，`StreamState.playing`（`core.rs:385-395`．Ctl `control.rs:567`，Own `tx.rs:362,998` が書く），Own の held の差し替え（`tx.rs:350-377,964-974`）．Mock は受付で取り込む（`lib.rs:1704-1712`）．行 40 本あまり（N47S5-33〜53，70〜80，90〜101，132，143，144，147，152〜158 など） |
| R15 | configure の拒否：その burst を記録から外し，残った短い頭を閉じ，後ろの piece の範囲を残す（RM-15 の例外，KL-38 `KNOWN_LIMITATIONS.md:148-153`，`staging.rs:500-533`） | KL-38 | — | 開いた短い最初の send → Underflow（再確認 1，`47s5-opus-rechecks.md:251`） | D′：configure は payload の発行の前提．拒否の後，発行していない piece を区切り直す（C1，§11.4）．X2 は代わりの案 | 記録の `remove_head` と Own の `configure_turn`（`tx.rs:581-611`）．Sol S2 と再確認 1・2 の major はここ．行 N47S5-89/140/141/145/146 |
| U1 | in-flight window W：now＋W の最初の sample より前の payload だけを渡す．send の大きさは W/4（32〜20 000 sample）（UR-23，`tx.rs:454-455,619`） | RM-16 の Stop の遅れの上限 | W = 10 ms は INFERRED．Module の窓 3 ms は 94〜99 % 成功，5・10 ms は 100 %（2026-10-28 の三つの記録，TXS の前，bench-results:233-241）．raw は 0.25 ms でも問題なし（:242）．測る群は段階 5 で preemption と一緒に消えた（`handoff.md` §2 の 2） | Stop が装置の buffer の深さまで遅れる（未測定，`plan/phase8/47-no-change-while-streaming.md` §4 の 5） | なし．値は Stop で測り直す．D′ では「封じ」の地平 | Own が実行．記録は `handover_horizon`・`stop_sample` で同じ W を使う．`ur_21_send_does_not_cross_the_in_flight_window`，`ur_23_m*` |
| U2 | 装置の区切り：開いたままの最初の send は 32 以上（repeat は並べる），閉じる最初の send は 4 へ詰める，続きの 1 sample の閉じは 2 へ，EOB は 2 以上，閉じた後の安全な開始は閉じの終わり＋1，全 send に時刻，開いた時に T3 flush（UR-22，UR-23，`tx.rs:614-805,849-887`） | 装置の規則（`KNOWN_LIMITATIONS.md` の「Device behaviour」の節） | **測った事実は VERIFIED**：開いた最初の send は 28 以下が Underflow・30 以上は可（補間 80 以上），閉じる最初の send は 1・2 MS/s で 4 未満が落ちる，閉じた後は +1（bench-results:322-353）．**32 と 4 をすべての rate に当てるのは安全側の選択**（`device.rs:651-663`．29 は未測定）で INFERRED | Underflow，捨てられる burst，次の burst の時刻が失われる（#70） | なし（装置の性質）．D′ では profile の値として区切りの関数に渡す（§6 の 6） | Own が実行し，記録が R3 で予測し，FakeDevice が独立に模す（`device.rs:641-734`）．行 N48S1-06/07/08/12/15/24，N48S3-01/02/04/07/08/10/35 |
| U3 | 保持の尾：継ぎがまだ来うる間は最後の sample を渡さずに持ち，J(e) の後の最初の turn に streams の lock の下で閉じる（UR-23，RM-16，`tx.rs:655-662,542-556`） | R4 の締め切りを e − 5 ms にする | 2026-10-28 の H5：600 burst で late 2〜4，underflow 2〜8（#78 と同じ形と読む，INFERRED．bench-results:245）．pending (d)1〜3．owner の未決「保持した最後の sample の期限」（`handoff.md` §3 の H1〜H5，保守表の 4） | 継ぎの締め切りが e − W − send の大きさに早まる | X5 は退ける（§11.9）．D′ では保持した尾は最後の buffer の操作で「発行した」に入り，渡すのは継ぎの最初の操作か，J(e)・Stop・打ち切りの `Close`（尾＋0）．Stop も尾を送る（§5.4.2，§5.4.8） | Own と記録の `continuation_open`・`close`（`staging.rs:252-261`）．S1 と M1(c) はここ．行 N47S5-25/56/57/133，`ur_21_m1_*` |
| U4 | dispatch での判定し直し：最初の send の前に今の時刻で late policy を判定し，動かした頭を他の予約と比べ，当たれば拒む．継いだ piece は動けない（D24）（RM-14，UR-21 `design/18-uhd-radio.md:243,247`，`tx.rs:458-465,1049-1126`） | RM-14「host の遅れ」 | —（host の遅れ） | 遅れた最初の packet を装置が捨て，後の時刻つきの packet が鳴る（部分的な burst，INFERRED） | D′：発行の時に状態機械が判定する（今と同じ時点，§5.4.7） | Own が決め，記録へ書き戻す（`move_start`・`move_untimed`・`drop_piece`，`tx.rs:1065,1083,1085,1104`）．F10 はここ．行 N47S5-31/61/72/88/123/124/129，N48S1-14/17 |
| U5 | 差し替えの取り込み：cursor が境目以前なら境目で，過ぎていれば R14 の late policy（UR-36，UR-38，`tx.rs:350-377`） | R14 | —（host が遅れた時だけ違いが出る） | — | D′（E2a）：取り込みは受付の時に決める（受付の時の発行した cursor は境目以前なので，いつも境目で取り込み，動かない）．Mock の今の形（MR-42）と同じ（§5.4.5） | Own と記録（`install`・`refuse`）と `playing`．F21，F39，S4 はここ |
| U6 | 打ち切り：underflow・sequence・時刻を過ぎた続き・失敗や短い send で device burst を閉じ，続けない．repeat は記録の上で cursor で終わり，保留の差し替えは取り消す（UR-22，`tx.rs:976-1020,1257-1262`） | UR-22，RM-37 | 遅れた続きは送られない（P6 starvation，2026-10-28，bench-results:223，VERIFIED）．H2（shake-out，:132） | 壊れた時間軸で続きが鳴る．記録が repeat を流れていると思い続け，stream が Stop まで塞がる（S4） | D′：結果と報告は名乗った操作・device burst にだけ当たる．最初の移行の記録の範囲は今のまま：有限の予約は範囲を残し，repeat は cursor で終える（§5.4.4） | Own と記録への 4 か所の書き込み（`tx.rs:990-1003`）．Mock に送信の故障の模型はない（M5）．S4 と再確認 1〜3 の major はここ．行 N47S5-136/139/151/153〜158 |
| U7 | F8 の recovery：EOB が不確かなら streamer を開き直して T3．予約した burst は続け，その間に時刻が来たものは late policy（UR-22，F8 (a)，`tx.rs:889-962`） | F8 (a)（owner） | 低い rate の T3 flush の Underflow（#78 の一つ目，bench-results:322-331，VERIFIED）．#75 は open | 閉じ方が不確かな後の burst が時刻を失う（#70 の形） | D′：`Reopen` の操作．payload の発行の前提（§5.4.6） | Own だけ（`TxChain.recovery`）．halt で `cancel_switch`（`tx.rs:955`） |
| U8 | configure の turn：新しい generation の burst の前に，device burst が閉じ，前の burst の終わりを過ぎ，recovery がない時に設定し，次の turn で late policy（UR-26 `design/18-uhd-radio.md:302-303`，`tx.rs:242-245,559-611`） | F2 ①，KL-35 | T5（VERIFIED） | 流れている burst の途中で設定が効く | D′：`Configure` の操作．payload の発行の前提．結果の後に Stop・今・予約を lock の中で見直す（§5.4.6） | Own（`configured` の印，`core.rs:372`）．書き手は一つ．拒否は印を消し，同じ generation の次の burst が設定し直す（`tx.rs:592-605`，UR-26 `:303`） |
| U9 | 報告の照合：streamer の generation，channel，tick（UR-28，`tx.rs:388-438,1170-1180`） | UR-28 | ACK は終わりの 1 tick 後（VERIFIED，bench-results:311,331）．UHD の非同期の報告は channel・時刻・event code・user payload だけを持ち，予約の id を持たない（[metadata.hpp](https://raw.githubusercontent.com/EttusResearch/uhd/v4.10.0.0/host/include/uhd/types/metadata.hpp) の `async_metadata_t`，VERIFIED） | 報告が別の burst に付く | D′：報告は device burst に照合してから状態機械へ（§5.4.4） | Own だけ |
| M1 | Mock は受付で決め，予約には in-flight の前線を使わない（MR-17 `design/09-mock-radio.md:143`） | parity | — | — | — | 記録を呼ぶ（`lib.rs:1577-1760`） |
| M2 | Mock の Stop の payload の切り：Stop の sample，窓の中で渡したはずの payload（最初の send が窓に入るものだけ），前の切りのうち遅いもの（MR-25 `design/09-mock-radio.md:171`，`lib.rs:1159-1210,1393-1420`） | parity（Own の handed cursor の模型） | — | Mock の Stop の終わりが Module と違う | D′（E3）：MR-25 の式を共有の区切りで計算する．Stop の sample を下限に，時刻どおりの real-time 実行役が発行したはずの cursor（profile の最初の send の値つき，send の大きさの上限なし）と前の切りの遅い方．振る舞いは変えない（§5.4.8） | Mock だけ．Own の渡し方の**三つ目の模型**．行 N48S1-23/25/29，N48S3-17/21 |
| M3 | Mock の差し替え：受付で取り込み（動かさない），装置の時刻で適用，故障の replay の順で決める（MR-42 `design/09-mock-radio.md:221`，`lib.rs:1704-1712,1791-1806`） | parity | pending (e) | F30（適用を決着させない），S3（loss より先に適用） | D′：理想の実行役が状態機械を装置の時刻の順に回す | 記録と Mock の step の順．行 N47S5-33/38〜40/51/93/94/132 |
| M4 | DeviceModel：開いた burst の中の SOB を `TIME_ERROR{unclosed_burst}` で落とす（MR-24 `design/09-mock-radio.md:169`，`crates/ezsdr-mock-radio/src/device.rs:9-38`） | Vision §58 の 15 | — | — | なし．SOB と EOB の開閉しか見ない（波形の正しさは見ない） | Mock だけ（独立の検査） |
| M5 | Mock に送信の故障（underflow，打ち切り）の模型がない（`grep -rn "Underflow\|underflow" crates/ezsdr-mock-radio/src/*.rs` は送信の故障を何も返さない．故障の種類は loss と受信の overflow・sequence だけ，`lib.rs:758-776`） | — | — | U6 の道は parity で試せない．S4 系はすべて UHD だけで固めた | D′：Mock の故障の予定から結果（short・失敗）を入れられる（足すかは別に決める） | — |

表から読めること（VERIFIED，上の行の写しの列）：一つの事実を二つ以上の所が持つ規則は R3，R4，R7，R8，R14，R15，U3〜U6，M2，M3．段階 5 の S1〜S4 と再確認の major は，すべてこの行に入る（§4）．一つしか持たない R13（値）と K1〜K3 からは，ずれの finding が出ていない．

## 4. 二つの写し

記録を書く所（VERIFIED，コード）：Ctl が受付（`control.rs:506-569`）と Stop（`control.rs:316`），loss と `Provider::stop` が `core.rs:589-604`，**Own が 15 か所**（`tx.rs:357,358,453,484,548,605,955,969,994,997,1065,1083,1085,1104,1105`）．Own はほかに `StreamState.playing` を 2 か所で書く（`tx.rs:362,998`）．Own はそのうえ自分の写しを持つ：`held`（`tx.rs:220`），`open`（:221），`chain: TxChain`（:230）．受け付けた burst は mpsc で Own に渡る（`control.rs:598`，`tx.rs:326-343`）．

| # | 状態 | 写し 1（記録） | 写し 2 以降 | 書き手 | 段階 5 のずれ |
|---|---|---|---|---|---|
| P1 | 受け付けた予約の集合 | `chains`（`staging.rs:181`） | Own の `held`・`open`．受付の後に mpsc で届く | Ctl，Own | S1（Own に届く前の close），F10 |
| P2 | 継ぎ点と開いた device burst | `join`・`joined`・`end`（`staging.rs:85-91`） | `TxChain::Open{next, tail}`（`tx.rs:104-112`） | Ctl，Own（`close`） | S1，M1(c)，再確認 3 |
| P3 | 装置の front | `front`（予測） | `TxChain.front`（実際，`tx.rs:121-125,177-180`） | Ctl，Own（`remove_head` 経由） | S2，再確認 1・2，計画の M1（148 µs） |
| P4 | Stop の後に残るもの | 範囲と `halt` | Own の `stop_now`・`end_open`，Mock の `tx_handed_end` | Ctl，`core::end`，Own（打ち切りの `stop_at`） | 計画の再確認の M2，major 1〜3，major A |
| P5 | 流れている repeat | 終わりのない chain | `StreamState.playing`（`core.rs:385`），Own の開いた repeat | Ctl（`control.rs:567`），Own（`tx.rs:362,998`） | S4，F21，再確認 1（一窓遅い），3（継ぎの隙間），minor A・B |
| P6 | 差し替えの結果 | `pending{installed, moved_to}`，`settled` | Own の held の差し替え，`switches` の行，Mock の取り込み | Ctl，Own（install・refuse・cancel_*），Mock | S3，S4，F21，F30，F39，F42 |
| P7 | 値の busy end | `until` | なし（C6 で保守的に数える） | Ctl，Stop | なし |

段階 5 のずれと，各案で残るか：

| ずれ | 列（短く） | 写し | A（今を固める） | A′（A＋X1・X2） | B（文字どおり） | D′ |
|---|---|---|---|---|---|---|
| S1 | A(40) を渡して尾を持つ → B と C が A に継いで受け付けられ，その TxCmd はまだ Own に届かない → Own が e で記録を読むと最後の継ぎは C → A を閉じる → B が遅れる（`47s5-sol-code.md:17-30`） | P1・P2 | 継ぎ点を全部持つよう直した | 同じ | controller が確定の時に決める | 予約の置き場所が一つ（E1）．mpsc がない．継ぐか閉じるかは lock の中で読む時刻の述語 |
| S2 | A(40)・B(10)・C(4) が続く → A の configure が拒まれる → B が開いた短い最初の send → C が遅れるか黙って失われる（:32-50，再確認 1・2） | P3 | `remove_head` の規則と KL-38 | X2 で起きない（ただし X2 は retry を覆わない，§11.4） | configure を確定の前に終える形なら起きない | 最初の移行は今と同じ．C1 で configure の後に発行していない piece を区切り直し，B と C は一つの device burst で鳴る |
| S3 | Mock の一つの step が loss（5500）と境目（6000）をまたぐ → `advance(u)` が先に差し替えを適用 → 行が applied（:52-66） | P6（Mock の中の順） | replay の順で直した | 同じ | Mock の作り次第 | 理想の実行役が状態機械を時刻の順に回す一つの規則 |
| S4 | repeat が underflow で打ち切られる → 記録は流れていると思い差し替え S を受け付ける → Own は取り込む cursor がなく，行が pending のまま残る（:68-84） | P5・P6 | 打ち切りで記録に 4 か所書く．その直しで再確認が 3 回 FAIL | 同じ | 報告を controller が一か所で | 打ち切りは名乗った device burst の結果・報告．その burst の予約を受けた cursor で終え，差し替えは送った事実で決める（§5.4.4，§5.4.5） |
| 再確認 1 | S2 の直しが 2〜32 sample の開いた最初の send を残した．S4 の止める点が一窓遅い（`47s5-opus-rechecks.md:251-253`） | P3・P5 | 直した | 前半は X2 で起きない | — | 区切りは一つの純粋な関数（E3）．止める点は結果の受けた cursor |
| 再確認 2 | piece が残るのに front を縮めた．cursor と境目が等しい打ち切りが applied（:257-258） | P3・P6 | 直した | 前半は X2 で起きない | — | 外した後は front を縮めず，発行していない予約から区切り直す．差し替えは境目を送ったかで決める |
| 再確認 3 | `join_open(b)` と差し替えの open の間の打ち切りが，repeat を記録に残す（:265） | P5 | 直した | 同じ | — | 境目は操作の始まりで，二段の取り込みがない |
| F10 | prune した頭を，止まっていた Own が遅れて動かし，追われない burst が後の予約に重なる（`47s5-selfreview.md:11`） | P1 | 動かした頭も試すよう直した | 同じ | 確定の時に決める | 発行していない予約だけが動く．動かすのは発行の時の状態機械 |
| F21 | 拒んだ差し替えが `playing` を書き換える（:17） | P5 | 取り込みの時に書くよう直した | X1 で道が一つ減る | — | `playing` は状態機械の中．境目を送った時だけ変わる |
| F30 | Mock が適用した差し替えを決着させない（:42） | P6 | 装置の時刻で `advance` を呼ぶよう直した | 同じ | — | 理想の実行役が装置の時刻で回す |

**なぜ起きたか（VERIFIED，コードと記録から）．** 記録は受付の時点の予測である．Own は装置で起きたこと（host の遅れ，configure の拒否，underflow，差し替えの取り込み）で実際を変え，その一つ一つを記録へ手で書き戻す．書き戻しの道が増えるたびに，書き忘れか，書く時点のずれが一つ増える．書き手が二つのスレッドなので，「記録には書いたが Own にはまだ届いていない」「Own は閉じたが受付は古い now を持っている」という窓もできる（S1，M1(c)）．今の答え（A）は，見つかるたびにその道を直して固めることで，段階 5 では Sol の 4 件と再確認の major 6 件をこの形で直した（`47s5-dispositions.md:128-129`）．

**D′ が消すもの，消さないもの（Sol §2・§4）．** 消すのは**同じ intent の独立した写し**（P1〜P6 の記録と Own の写し，mpsc の配送の窓）である．**予定と実行の結果の違いは消えない**：short send，例外，configure の拒否，underflow は I/O の性質で，どの形にも残る．D′ はそれを一つの状態機械の中の実行の状態（§5.4.2）として持ち，結果を発行した操作に一度だけ当てる．「報告で何も変わらない」ほど単純にはならないが，これは二重の実装ではなく I/O の不確かさの表現である．

**今のコードの不具合の見込み（INFERRED，第 2 版への Opus の再確認 §2 の参考．コードを読んだ推論で，走らせていない）．** 二つの写しの同じ形が，今もう一つ残っている．

```text
repeat A の send が in flight（Own は tx_send の中）
Ctl：Stop@t を受付（halt と TxCmd::Stop）．続けて予約 B（start ≥ B_stop）を受付（B の chain と playing = B，TxCmd::Burst(B)）
Own：A の send が失敗か short，または underflow の報告 → abandon．A は repeat なので stop_at(cursor, 0)（tx.rs:993-999．stop_at は core.rs:399-403 で halt を呼ぶ）
     → halt が start ≥ cursor の chain をすべて消す（staging.rs:346-349）．B の chain も消え，playing も None になる
Own：次の turn で TxCmd::Stop と TxCmd::Burst(B) を取る → B は Own の held に入り，鳴る
→ 記録に B がないので，B に重なる予約が受け付けられ，流れている B への switch は Rejected になる
```

似た窓：Own が mpsc を drain した後（`tx.rs:285-292`），step で now を読む前（`:451`）に Stop が来ると，その turn は Stop を知らずに `at_or_after(now＋W)` まで渡すので，渡した所が B − 3 を超えうる（INFERRED）．どちらも `mech:planned-state-read-as-actual` で，issue は orchestrator が立てている（2026-11-08）．D′ では，前者は E2a で構造的に起きない（結果と報告は自分の op の device burst にだけ当たり，Stop の後に受けた B の範囲を消さない，§5.4.4）．後者は E1（Stop を予約と同じ受付の順の列に入れ，Own はその列を lock の中で順に読む，§5.4.4 の 6）と，発行の now を lock の中で読むこと（§5.4.3）で閉じる．

## 5. 案

### 5.1 A：今の形を固める（と A′）

二つの写しを残し，一致をテストで固める．差分の harness（同じ操作の列を記録と FakeDevice の上の Own に流し，front・閉じた所・結果を比べる）を足せる．stat `tx_close_past_record_front` はすでに on-time と parity の試験で 0 と固めている（Q-6，`47s5-test-plan.md:755`）．

- 利点：移行の手間がない．2026-11-09 の実機がそのまま今の形を確かめる．
- 欠点：harness はずれを後で見つけるだけで，新しい道（8b の揃った開始，8c の Replay，Phase 10 の link からの送信）が加わるたびに書き戻しの道が増える．段階 5 だけで N47S5 の行を 185 本（うち `staging.rs` に 70 本）足した（`mutations.json`）．機構は 3 回を超えたまま残る．
- **A′（A に X1・X2 を足す）**：R15 と U5 の Drop の枝が消え，S2 系は一部起きなくなる（X2 は configure の retry を覆わない，§11.4）．P1・P2（S1・M1(c) の窓）と P5・P6（S4 系）は残る．

### 5.2 B：controller → 一方向の executor（owner の二段の形，2026-11-07）

| B の制約 | 簡単になるもの | 失うもの | D′ での扱い |
|---|---|---|---|
| (b1) controller が未来の予約を可変で持つ（並べ替えてよい） | KL-37 の「渡したかもしれない」が時刻の定義になる | なし | 同じ |
| (b2) 地平 H（≥ W）に入ったものだけ確定する | Stop・差し替え・渡しを一つの地平で決める | 保持の尾がなければ，継ぎの締め切りが確定の時（≈ e − W − send の大きさ）に早まる | H = W．32 を超える chain の最後の sample を運ぶ操作は J(e) まで発行しない（今の保持の尾と同じ締め切り） |
| (b3) executor は一方向の FIFO．割り込まない，問い合わせない，出来事だけ返す | 持ち主が決めることがなくなる | 予約の単位で確定すると，長い burst の途中の Stop と差し替えが「割り込み」になる．割り込まなければ Stop の遅れが burst の残りの長さになり RM-16 を破る | 発行の単位を sample の区間（一つの操作）にし，in-flight は一つ．Stop と差し替えは発行していない区間だけに効くので割り込みは要らない |
| (b4) 確定の時に遅れていれば controller が late policy を決める | dispatch の判定し直しと記録への書き戻し（U4）が消える | なし | 判定は**発行の時**（今の dispatch と同じ時点）．configure の後にも見直す（§5.4.6） |
| (b5) J(e) の「継ぐか閉じるか」を確定の時に決める | `close(e)` と受付の競争（M1(c)）が消える | なし | 同じ（受付の now を lock の中で読む） |
| (b6) 差し替えの移動を確定の時に決める | cursor での取り込みの競争（U5，S4 系）が消える | なし | 境目を運ぶ操作の発行の時に決める |
| (b7) 確定の後の stall はただの host の遅れ | 単純 | 確定の後に遅れて動いた長い頭が，まだ確定していない後の予約に当たる時，今は「動いた方を拒む」が，B では後の予約が遅れになる | D′ では発行していない予約は動かせる（判定は発行の時）．発行した後の stall は装置が判じる（§5.4.7．今と同じ） |
| (b8) 代価：時刻で回る controller の loop | — | controller が実時間で確定を出さないと executor が飢えて underflow する．uhd-control の loop は 1 ms（`control.rs:35`）で実時間の優先度がなく（UR-23 は uhd-tx だけ），同じ loop で装置の読み（`ref_locked`・`time_now`，`control.rs:616-644`）もする．**ただしこの間 streams の lock は持たない**（`core.rs:252-256`「never held across a device call」，`check_device` は streams の guard を取らない，VERIFIED）．危険は lock の待ちではなく，controller 自身の遅れで確定が遅れること（INFERRED） | loop を足さない．uhd-tx の今の実時間の loop が操作を引く（pull） |
| (b9) 代価：地平の値 | — | 新しい profile の値と測定 | 地平は W そのもの（Own がもう使っている `window_cut`，`tx.rs:454-455`） |

B の意味（一人の持ち主，確定の地平，一方向の実行役）は正しい．外の先例も同じ形をとる（§5.11）．文字どおりの形（別の controller のスレッドが押し出す）は (b3)・(b8) で困る．

### 5.3 C：持ち主の実際だけを真にする（記録を消すか，受付の拒否に要る分だけにする）

- 受付の答えが Own の進み（host の時刻）に依るので，Mock（stepped で決定的）と同じ答えにならない．parity（Vision §59）と #79 の決定（両 Provider が受付で同じ答え）が崩れる．
- O2（submit の返事の `Rejected`）と原則 4（受け付けた予約との衝突は拒否）には，受付の時点で受け付けた予約の集合と，その区切りが要る．「拒否に要る分だけの記録」は今の記録そのものである．
- Mock は Own の進みの模型を持つことになる（M2 の模型がすべての判断に広がる）．
- **退ける．**

### 5.4 D′：一つの状態機械と，一つずつの操作（決定，owner 2026-11-08）

第 1 版の D に Sol の方向レビューの 9 件を入れた形．各 finding の答えは次の小節の設計の文に入れた（finding の番号は Sol の番号）．

#### 5.4.1 何を一つにし，何が残るか

- 一つにするもの：受け付けた予約（波形の `Arc`，late policy，時刻なしか，差し替えか，値の snapshot と generation．今の `Held` の中身，`tx.rs:28-43`），区切り（device burst の identity，開いている device burst の次の sample，保持した尾，閉じた後の安全な front），Stop の境と範囲，保留の差し替えと流れている repeat，**発行した操作とその結果**．置き場所は送信 stream ごとの一つの状態機械で，今の `StreamState.busy` と `playing`（`core.rs:383-385`），Own の `held`（`tx.rs:220`），Mock の `held` と `busy`（`crates/ezsdr-mock-radio/src/lib.rs:208,216`）を置き換える．予約は開始時刻を key にした ordered map で持つ（§11.6 の owner の注：受け付けた footprint は重ならないので，前後の一つずつを見れば重なりが分かる．INFERRED，設計の段で確かめる）．受け付けた Stop も同じ状態機械の入力で，予約と同じ受付の順で処理する（§5.4.4 の 6）．
- 実行役（uhd-tx）に残るもの：装置の事実だけ．streamer の handle と generation，報告の照合の材料（今の `unacked`，`tx.rs:388-438`），送りかけの操作．
- スレッドの形は変えない．Ctl が受付を，uhd-tx が発行・結果・報告を，同じ streams の lock の下で状態機械に渡す．

#### 5.4.2 三つの概念と，sample の状態（finding 1，7）

1. **時刻による受付の封じ（sealing）**．送信の sample k は，装置の時刻 t に instant(k) < t＋W のとき封じられている．これは k < at_or_after(t＋W) と同じで，今の `window_cut`（`tx.rs:454-455`，`k < window_cut` で渡す，`:504`）と `handover_horizon`（`staging.rs:615-617`，`c.start < horizon`，`:566`）と同じ strict な形である（VERIFIED）．状態を持たない時刻の述語で，受付はこれだけを読む．封じた sample の予定は受付では変えない（その前に予約を入れない，動かさない，その上に継がない）．例外は §5.4.10 の J(e)．
2. **発行（issue）**．状態機械が実行役に操作を一つ渡すこと．操作は不変の token：`Send{op, burst, k0, 範囲, 時刻, SOB, EOB, 詰め物}`，`Close{op, burst, at}`（2 sample の EOB），`Configure{op, generation, 値}`，`Reopen{op}`（recovery：開き直しと T3 flush）．op の id は**装置を呼ぶ前に**状態機械に登録する．一つの stream に，発行して結果を受けていない操作は高々一つ．発行できる条件は §5.4.6・§5.4.10．
   **保持した尾**（M1）：継ぎがまだ来うる間に burst の最後の buffer を渡す時，今の Own は最後の 1 sample を渡さずに持つ（`hold`，`tx.rs:658-662`）．D′ ではその最後の buffer の `Send` が [k0, e − 1) を運び，尾 e − 1 は「発行した」に入るが，まだ装置に渡していない唯一の sample として device burst に残る．尾を渡すのは同じ device burst の次の操作だけ：継ぎの最初の `Send` の前の 1 sample の send（今の `tx.rs:663-683`），または `Close{tail＋0}`（J(e) の閉じ，Stop，今の `end_open` → `close_device_burst(tail, …)`，`tx.rs:823-845,849-866`）．打ち切りの閉じは尾を捨てて 0 を二つ送る（今の `abandon` → `close_device_burst(None, …)`，UR-22「discards its held tail」）．
3. **実行の結果（result）**．実行役が装置を呼んだ後に返す `{op, outcome}`．outcome は `Accepted{n}`（n sample を受けた．範囲の長さなら完了，短ければ short），`Failed{lost}`（例外．受けた数は分からない．今も `tx.rs:736-743` は「Whether UHD sent packets before the error is unknown」として保守的に扱う），`Configured`・`ConfigureRefused`，`Reopened{ack}`・`ReopenFailed`．

送信の sample と予約の状態は四つを区別する：

| 状態 | 意味 | だれが決めるか |
|---|---|---|
| 封じた（sealed） | 受付ではもう変えない | 時刻の述語（状態なし） |
| 発行した（issued） | その sample を含む `Send` が登録された．取り消せない．保持した尾もここに入る | 状態機械（`next`） |
| 送った（sent） | その `Send` の結果がその sample を受けた．保持した尾だけは，尾を渡す操作の結果で送ったになる | 結果（`complete`） |
| 鳴った（applied） | 送った sample を装置の時刻が過ぎ，それより前の打ち切り（underflow の tick）がない | 状態機械（時刻と報告） |

**Kernel の burst の記録（M1 で直した）．** 今の BurstTracker は，send が全部受けられた後にだけ進む（`tx.rs:718,791`）．ただしその block の長さは `len: count` で（`:764`），保持した尾を含む．尾を残して渡す send（`sending = count − 1`，`:662`）でも，Kernel の burst はその時に尾まで数えて `END_OF_BURST` で終わる（VERIFIED）．つまり今の記録は，尾を装置に渡す前に尾を数える．第 2 版の「全部を受けた send の後にだけ進む」は，この点で正確でなかった．

D′ の最初の移行はこれを変えない：Kernel の burst の記録は，最後の buffer の `Send` の結果の時に尾まで数える（block の数と長さは今と同じ）．尾を後で捨てる打ち切りの時は，記録が 1 sample 多く数える（今と同じ，INFERRED）．尾を渡す操作の結果まで数えるのを遅らせると block の数が変わる（SC-28 の記録の中身）ので，するなら別の契約の変更として owner に諮る．`Failed` の範囲は「送ったか分からない」で，数えない（今と同じ）．

#### 5.4.3 状態機械と遷移

遷移は四つで，どれも (状態，入力，装置の時刻) の純粋な関数である．外へ出す event（`TIME_ERROR`，`COMMAND_REJECTED`，`switches` の行，記録の行）は値として返し，lock を離してから出す（今の `book_burst` も lock の外で出す，`control.rs:572-592`，VERIFIED）．

- `receive(action, now) -> 答え`（Ctl，Mock）：今の `accept`・`place`・`region_move`・`switch`・Stop と同じ答え．「封じた」は §5.4.2 の述語．
- `next(now) -> Option<Operation>`（実行役）：発行できる操作を一つだけ返す（§5.4.6，§5.4.10）．遅れの判定（RM-14，UR-21）はここ（§5.4.7）．
- `complete(op, outcome, now)`（実行役）：結果を，その op に一度だけ当てる（§5.4.4）．
- `report(generation, channel, tick, code, now)`（実行役）：非同期の報告を，照合した device burst に当てる（§5.4.4）．

実行役（uhd-tx）の loop：lock → **lock の中で now を読む** → 報告を `report` → `next(now)` → unlock → 装置を呼ぶ（`Send` は `tx_send`，`Configure` は `configure_tx`，`Reopen` は開き直しと T3）→ lock → `complete` → unlock → event を出す．`next` の now を lock の中で読むので（m2），発行は lock の順で受付と並び，DeviceAuthority の now は単調（`authority.rs:143-144` の `fetch_max`）だから，Stop の受付より前の発行は Stop の now より前の now で封じた sample だけを運ぶ．だから §5.4.8 の c_I ≤ B − 3 が成り立つ．今は Own が lock の外で now を読む（`tx.rs:451`）ので，この保証はない（§4 の似た窓）．

MockRadio は**理想の stepped 実行役**として同じ状態機械を回す：`step(until)` の中で故障と Action を時刻の順に処理し，操作が発行できる最初の瞬間に `next` を呼び，結果は即座に `Accepted{全部}`（Mock に送信の故障はない，M5）．block（MR-15）は送った範囲から，最後の sample が過去になった時に出す（MR-15 `design/09-mock-radio.md:133-134`）．stepped 実行役は遅れない（host がない）ので，発行の時の遅れの判定をしない．Mock の遅れは今と同じく受付で決まる（MR-17）．stepped 実行役について三つを決める：

- **装置の最初の send の最小（U2 の 32 と 4）は stepped 実行役の発行と block の出し方を縛らない**（M4）．これは X3x0 の packet の規則で，profile の値として real-time の実行役の区切りにだけ渡す．Mock が装置の規則を使うのは，Stop の切りで「時刻どおりの real-time 実行役ならどこまで渡したか」を模す時だけで，それが今の MR-25 の第二項である（§5.4.8）．
- **configure**（m4）：Mock は burst の値を，その burst の最初の sample の瞬間から使う（`open_held`，`lib.rs:972-985`．B6，`47-no-change-while-streaming.md:123`）．拒否も装置の時間もないので，`apply_due` の待ち（前の burst の終わりまで，`tx.rs:242-245`）は stepped 実行役には当てない．値は stream の予約の終わりが受付以前の時だけ受けるので（RM-21），前の burst と重なる値は Mock でも起きない．
- **いつから共有の形を使うか**（m3）：Mock は今，受付で `Bookings` を直接呼び（`lib.rs:1577-1760`），自分の `held`（`:208`）を持つ．E1 で予約と Stop を UHD と同じ状態機械の列に移す（Mock の step がその列を受付の順に読む）．E2a で Stop・差し替え・結果の遷移を同じ状態機械に任せる（結果はすぐ `Accepted{全部}`）．block の出し方（MR-15）と Stop の切りの模型（`tx_handed_end`）は E3 まで今のまま．

#### 5.4.4 操作の identity，結果と報告の当て方，受付との順序（finding 5）

1. op の id は装置を呼ぶ前に登録する（§5.4.2）．in-flight は一つなので，結果は必ず登録した op に当たる．二つ目の結果は来ない（来れば debug で落とす）．
2. 結果は一度だけ当てる．当てる範囲は**その op の device burst だけ**．short・失敗・underflow の報告で打ち切る時（M5）：
   - **device burst**：受けた cursor（`k0 + n`，失敗なら op の終わり）で閉じる（2 つの 0，尾は捨てる）．閉じの front は cursor＋3．
   - **記録の範囲（最初の移行は今のまま）**：有限の予約は受け付けた範囲を残す．今も有限の burst の打ち切りでは範囲を変えない（`tx.rs:986-999` は repeat の時だけ `stop_at` を呼ぶ）．だから打ち切りの後でも，その範囲に重なる予約は受付で拒まれる（再確認の列：A(100000)@k の 3 回目の send が失敗 → raw の C@k＋50000 は今も D′ でも Rejected）．repeat はその cursor で終える（今の `stop_at(cursor, 0)` と `playing = None`，`tx.rs:993-999`）．ただし終えるのは**その op の device burst が運んでいた repeat の予約だけ**で，「cursor 以後のすべての予約」ではない（§4 の今の不具合の見込みを E2a で消す所）．範囲を縮める（打ち切りの後の有限の範囲を空ける）のは原則 4 の別の契約の変更で，最初の移行に入れない．
   - **まだ発行していない継ぎの piece**：取り消さない．後の burst と同じく，自分の発行の時に新しい device burst の SOB として late policy で判じる（閉じの front より前なら late．今の `later_bound` と `late_policy_front_outcome`，`tx.rs:1049-1060`，原則 4 の「装置の状態」）．
   - **差し替え**：その burst の中で cursor 以後に境目を持つ保留の差し替えを決着させる（§5.4.5）．後の device burst の予約と差し替えは変えない．
3. 同じ実行役は，結果を当ててから次を発行する．
4. 結果を待つ間の受付は状態機械に入る．受付は発行した操作の範囲を変えられない（封じてある）．Stop は発行していない区間だけを取り消し，閉じる位置を発行した区間の終わりにする（§5.4.8）．古い結果は「現在の repeat」ではなく自分の op の burst に当たるので，Stop の後に受けた予約 B を消さない（Sol の列：A が in flight → Stop → B を受付 → A が失敗 → A の burst だけが終わり，B はそのまま）．
   **Stop の受付の答えはすぐ効く**（N1）：Stop の受付の時の記録の変化（`halt`：t 以後に始まる予約を受付の表から外す，残る chain の front を B で切る，範囲 [t, B) を足す，`control.rs:312-316`，`staging.rs:346-367`）は，今と同じく受付の時に効く．in-flight の操作の予約も含めて，Stop の後の受付はこの表で答える（例：Configure が in flight の X を Stop が外した後の `send R@B` は今と同じく受け付けられ，時刻なしは B に置かれる）．結果を待つのは，in-flight の予約の**報告**（`COMMAND_REJECTED` か note）と，それを**発行の列から除くこと**だけである．
   **in-flight の操作の予約と Stop の報告**（m1，答えは一つ）：in-flight の結果を先に当て，その後で Stop の取り消しの報告と，発行の列からの除去をする．これは今の順序と同じで，Own はその turn の結果を処理してから，次の loop の初めに `TxCmd::Stop` を取る（`tx.rs:285-292`）．
   - `Configure` が in flight の予約 X：`ConfigureRefused` なら X は `COMMAND_REJECTED`（kind `tx_burst`，X の id．今の `tx.rs:603-606`）で，Stop の取り消しの note は出さない（X はもうない）．`Configured` なら，その後に Stop が X を取り消す（note だけ．`cancellation_reports(Stop)` は false，`crates/ezsdr-radio/src/lib.rs:123-126`，RM-16）．
   - `Reopen` が in flight：`ReopenFailed` なら stream の clock が終わり，待っていた予約は `ClockEnded` で取り消される（`COMMAND_REJECTED` が出る，今の `tx.rs:933-936,948-951` と `cancellation_reports(ClockEnded)` が true）．`Reopened` なら，その後に Stop が待っていた予約を取り消す（note だけ）．
   - `Send` が in flight：結果（完了・short・失敗）を当ててから，Stop がその device burst を閉じる（§5.4.8）．
5. 古い報告と重なった報告：UHD の報告は予約の id を持たない（U9）ので，今と同じく streamer の generation，channel，tick で device burst に照合する（`tx.rs:388-438`）．古い generation の報告（`sync_streamer`，`tx.rs:1170-1180`）と，もう閉じた burst の報告は記録だけして状態を変えない．同じ遅れの重なった `TIME_ERROR` は今と同じく一つにまとめる（`late_reported`）．**受けた数の分からない例外**（`Failed`）は別の扱いで，device burst は op の終わりまで渡したとみなして閉じ（今の `abandon(true, device_count, …)`，`tx.rs:743`），payload は数えず，その範囲に境目を持つ差し替えは `uncertain`（§5.4.5）．
6. **Stop と予約は受付の順の一つの列**（M3）：今は `TxCmd::Burst` と `TxCmd::Stop` が同じ mpsc に受付の順で入り（`control.rs:309-323,598`），Own はそれを FIFO で取る（`tx.rs:285-292`）ので，Stop の後に受けた予約は Stop の後に処理される．E1 で予約だけを共有の列に移し，Stop を mpsc に残すと，この順が崩れる：
   ```text
   Own：mpsc を drain する（Stop はまだない）
   Ctl：Stop@t（halt，TxCmd::Stop を送る）→ 時刻なしの repeat R を Stop の境 B に置き，共有の列に入れる
   Own：lock の中で列を読む → R が次になり，A を B で切って R を開く
   Own（次の turn）：TxCmd::Stop → stop_now → R の device burst を閉じる
   → Stop の後に受けた R が，その Stop で打ち切られる（RM-16「a later burst is transmitted as usual」，design/07-radio-model.md:168 に反する）
   ```
   だから E1 は Stop も同じ状態機械の入力にし，予約と Stop を受付の順の一つの列（受付の時に lock の中で付ける順の番号）に入れる．Own はその列を lock の中で順に読み，Stop より後の予約を Stop より前に扱わない．`Provider::stop` は今も Run の終わりを各 stream に予約してから thread を止める（UR-26，UR-38，`core.rs:589-604`）ので，同じ列の入力になる．thread への `Shutdown` の知らせだけが mpsc に残る．clock の origin 以前の `Provider::stop` で取り消される予約の理由は，今は `end_clock` と `Shutdown` のどちらが先かで `ClockEnded`（`COMMAND_REJECTED`，`tx.rs:1158-1163`）か `ProviderStop`（note だけ，`tx.rs:1290`）に分かれる．E1 では列の順で `ProviderStop`（note だけ）に決める（`Provider::stop` の取り消しは Stop と同じく報告しない．`ClockEnded` は Reopen の失敗で clock が終わる時だけ）．

#### 5.4.5 差し替えの境目の列（finding 1）

区切りは差し替えの境目 b で必ず操作を切る（今も held の差し替えの start で切る，`tx.rs:615-640` の `held_cut`）．だから b はいつも操作の始まりで，b を運ぶ操作は「b から新しい波形」の続き（SOB なし，時刻つき）になる．

**取り込み（`installed`）の写し先（M2）．** 今の Stop の規則は `installed && boundary < stop`（`staging.rs:358`）で，`installed` は UHD では Own が mpsc から差し替えを取った時に立ち（`tx.rs:350-377` → `staging.rs:389-402`），Mock では受付の時に立つ（`install(id, now_k, false)`，`lib.rs:1710`，MR-42）．D′ には mpsc がない．**E2a では取り込みを受付の時に決める**：受付（lock の中）で `install(id, c_I, moves)` を呼び，c_I はその時の発行した cursor とする．発行した sample は封じてあり，発行の now は受付の now 以前（lock の順と単調な now，§5.4.3）なので，c_I ≤ at_or_after(受付＋W) ≤ b − lead で，c_I はいつも b 以前にある（等号は lead 0 の `ideal` だけ）．`install` は `cursor_k <= boundary` なら b で取り込む（`staging.rs:391`）ので，差し替えは受付の時に b で取り込まれ，動かない．これは Mock の今の形（MR-42）と同じで，Stop の規則（`installed && b < B`）もそのままなので，最初の移行で UHD と Mock の Stop と差し替えの outcome は今の規則のとおりにそろう．

- **決定的になる差**：今の UHD は，Own が差し替えを取る前に Stop が来ると `installed` が立っておらず，`halt` が取り消す（`cancelled`）．同じ列で Mock は `applied`（b < B なら）．この UHD と Mock の時刻しだいの差は，E2a で Mock の今の答えに決まる．
- **host の遅れの移動（S12，UR-38）**：今の移動は「Own が差し替えを取った時に handed cursor が b を過ぎていた」時に起きる．受付の時に取り込めば c_I は b 以前なので，この道は Provider からは届かなくなる（INFERRED）．b を運ぶ操作の発行が遅れた時は，b は続きの操作なので，今の続きの規則（時刻を過ぎた続きは打ち切り，UR-22）で repeat が打ち切られ，差し替えは取り消される（RM-37）．`install` の移動の枝（`staging.rs:393-395`）と UR-38 の「A cursor past the boundary … moves」（`design/18-uhd-radio.md:298`）は E2a で届かなくなるので，**E2a で消す**（AGENTS §9 Maintainability 4．X1 は Drop の枝だけを消した）．

| 列 | 結果 |
|---|---|
| b の前の操作が `Accepted{全部}` → b の操作を発行 → `Accepted{n ≥ 1}` → 装置の時刻が b を過ぎる | `applied`．underflow の tick は送った終わりなので，b を送った後の underflow は取り消さない（RM-37「before handing the sample at b」と同じ） |
| b の前の操作が short（`k0 + n < b`） | その device burst は `k0 + n` で終わる．b の操作は発行されない → `cancelled`．Sol の列（b をまたぐ前に short → 新しい波形が一度も渡らないのに applied）は起きない |
| b の前の操作が `Failed` | その device burst は op の終わり（= b）まで渡したとみなして閉じる．b の操作は発行されない → `cancelled`（今も cursor = b の打ち切りは取り消す，`staging.rs:426-432`） |
| b の操作が `Failed` | b を運ぶ範囲が「送ったか分からない」→ `uncertain`（新しい outcome．契約の変更 C2）．**今は `applied` になる**（例外は op の終わりまで渡したとみなす `tx.rs:743` と，cursor より前の境目を取り消さない `cancel_switch_from`，`staging.rs:426-432`，から INFERRED） |
| Stop が b の操作の発行の前に来る | 実際に発行した所で閉じる（§5.4.8）．b は発行されない → `cancelled`．**今は B が b より後なら `applied`**（`staging.rs:358`：installed で b < B なら適用．Own は渡していない held の差し替えを取り消すので新しい波形は出ない．計画の再確認 m4 と Q-2 の裁定「applied は記録が適用したという意味」，`47s5-opus-rechecks.md:79,90`） |
| 流れている repeat がまだ発行されていない時に差し替えを受け付ける（c_I がその repeat の device burst の中にない．新しい generation の repeat の Configure が遅い，Own が W 以上止まった等．記録は start ≤ now の repeat を流れているとみなす，`staging.rs:377`） | 今と同じく **refused**（`switches` の行 refused と `COMMAND_REJECTED`）．今の Own は cursor がない時に差し替えを拒む（`tx.rs:351,357-359`，S4）．E2a は受付の時にこれを判定する（N2）．取り込んだ後で repeat が発行の時に動くか落ちる道は，この判定で起きない |
| host の遅れで b の操作の発行が b の時刻を過ぎる | b を運ぶのは続きの操作なので，時刻を過ぎた続きとして repeat を打ち切る（UR-22）→ 差し替えは `cancelled`（RM-37）．上の S12 の移動はここでは起きない |

最初の移行（E2a）は今の outcome の規則（受付で取り込む，Stop は B と比べる，例外は applied）をそのまま記録し，上の表の正直な outcome（`cancelled`・`uncertain`）は契約の変更 C2（§11.10）で両 Provider を同時に変える．C2 で MR-42 の文（受付で取り込む）も改める．Mock は送信の故障を持たないので，C2 で Mock が変わるのは Stop の行だけである．

#### 5.4.6 configure と recovery は payload の発行の前提（finding 2，4）

- `next` は，新しい generation の burst の最初の `Send` の前に `Configure{g}` を返す．条件は今の `apply_due` と同じ：device burst が開いていない，装置の時刻が前の burst の終わり（詰め物と保持の尾を含む）以後，recovery がない（`tx.rs:242-245`）．条件が満ちない間は何も返さない（待つ）．
- `complete(Configured)` の後，lock の中で次の `next` が見直す：その間に来た Stop（来ていれば burst を取り消し，`Send` は出さない），今の時刻（late policy をここで判定．SendAsap は動かし，Drop は落とす），予約．これは今の「configure の turn の後，次の turn で command を処理し late policy を判定する」と同じ順序である（`tx.rs:577-609` の後に `:503-531` の dispatch，UR-21 `design/18-uhd-radio.md:247`，UR-26 `:302`，VERIFIED）．**第 1 版の D の「確定 → configure → 送る」は，configure が遅れると確定した開始の SendAsap の移動を失う順序だった（Sol finding 2）．D′ はこれを今の順序に戻した．**
- `ConfigureRefused` はその予約を `COMMAND_REJECTED` で拒み（今と同じ．configure の間に Stop が来ていても同じで，Stop はその後で残りを取り消す，§5.4.4 の 4），印を消す（`record_configured(key, None)`，`tx.rs:592-597`）．同じ generation の次の burst は設定し直す（UR-26 `:303`，`tx.rs:559-567`）．最初の移行では，後ろに継いだ piece は今と同じく KL-38 のまま．**契約の変更 C1（§11.4 の決定）**：configure は payload の前に終わるので，拒否の時点で後ろの piece は一つも発行されていない．状態機械は拒まれた予約を外し，残りの piece を一つの chain として区切り直す（例 B(10)＋C(4) は 14 sample の閉じた device burst．設定し直しの後に時刻どおりに出る）．これで KL-38 の遅れはなくなる．Sol の列（A が拒まれ，同じ g の B(40)＋C(10)＋D(4) でも B の configure が拒まれる）も，C と D が区切り直され，C の前の configure の retry に通る．
- `Reopen`（recovery）も前提：EOB の閉じが不確か（今の `mark_reopen_needed`）なら，`next` は `Reopen` を返し，終わるまで payload を発行しない（今の `send` も `needs_reopen` の間は送らない，`tx.rs:615-617`）．`Reopened{ack}` の後の front は ACK の tick（今の `tx.rs:907-911`），遅れた予約は発行の時に late policy（F8 (a)）．

#### 5.4.7 発行の後の stall と，変わる契約（finding 2）

発行（lock の中）と装置の呼び出し（lock の外）の間，または呼び出しの中で実行役が止まることは防げない．D′ は今と同じ扱いにする：

| 発行した操作 | 実行役が遅れた時 | 今と同じか |
|---|---|---|
| 時刻つきの SOB の `Send` | 装置が遅れた packet を `TIME_ERROR` で落とす．実行役は `TIME_ERROR{late_at_device}`（UR-28）を報告し，その後の時刻つきの続きも装置が落とす | 同じ（今も `dispatch_start` と `tx_send` の間に同じ窓がある） |
| 続きの `Send` | 遅れた続きは装置が落とす（P6，VERIFIED）．報告で device burst を打ち切る（UR-22） | 同じ |
| 時刻なしの予約の `Send` | 置いた開始の時刻を持つ（全 send に時刻，UR-22）ので，SOB と同じく装置が判じ，`TIME_ERROR{late_at_device}` が出る．Provider は時刻なしを遅れと判じない（RM-15）が，装置の報告は出る | 同じ（INFERRED：今の UR-21 の時刻なしの文は Provider の判定だけを言う） |

**最初の移行で変わる契約はない**：遅れの判定は発行の時（今の dispatch と同じ時点），configure は payload の前（今と同じ），Stop は発行した所で閉じ，保持した尾も送る（今の handed cursor と `end_open` と同じ），in-flight の結果は Stop より先に当てる（今の順），打ち切りの後の記録の範囲は今のまま（有限は残し，repeat は cursor で終える），差し替えは受付で取り込み outcome は今の規則，Kernel の burst の記録は尾を最後の buffer で数える（今と同じ）．四つだけ，今の UHD の時刻しだいの差が決まる（§5.4.5，§5.4.4 の 6）：Own が差し替えを取る前に repeat が打ち切られた時に今は余分に出る `COMMAND_REJECTED`「the repeat switch was not taken」（`tx.rs:331-333`）が E2a では出ない，clock の origin 以前の `Provider::stop` で取り消される予約の理由が E1 で一つに決まる（§5.4.4 の 6），Own が差し替えを取る前に来た Stop（今は `cancelled`，E2a では境目が B より前なら `applied`）と，host の遅れで境目を動かす道（UR-38 の S12．E2a では届かず，`moved_to` は `null`，INFERRED）．どれも RM-37 の規則（境目，Stop は B と比べる，late policy での移動）は変えない．変わるのは UR-38 の取り込みの時点の文（Own が handed cursor で取り込む → 受付の時に発行した cursor で取り込む）で，E2a に入れる．**変える契約は契約の変更の段階に集める**（§12）：C1 KL-38 の撤回（RM-15 の例外，UR-26，KL-38），C2 差し替えの正直な outcome（RM-37，UR-38，MR-42，`switches` の行），そして owner が後で承認した時だけの C4 Stop (a)（RM-16，UR-23，MR-25，KL-13）と C5 短い頭への継ぎ（RM-15，UR-22，UR-23，KL-36）．N0 の X1（RM-12・RM-13，届かない枝）と §11.2 の文（振る舞いは変えない）は移し替えの前に入る．

#### 5.4.8 Stop（finding 3）

- **最初の移行（§11.8 (b)，owner が D′ の決定で選んだ）**：Stop の受付（lock の中）で，状態機械は発行した区間の終わり c_I（in-flight の操作の範囲と，保持した尾を含む）を知っている．c_I 以後の予約と区間をすべて取り消し（RM-16 の「渡していない held の burst は取り消す」，`design/07-radio-model.md:168`，`tx.rs:1149-1156`），c_I で閉じる `Close` を，in-flight の結果の後に発行する（§5.4.4 の 4）．**保持した尾があれば，`Close` は尾＋0 を運ぶ**（M1）．今の Stop も `stop_now` → `end_open(true)` → `close_device_burst(tail, …)` で尾と 0 を一つの EOB の send で送る（`tx.rs:823-845,849-866`）．だから再確認の列（A(1000) の 999 を渡して尾を持つ → J(e) の前に Stop）でも，A の 1000 sample がすべて出て，Mock（MR-25 は A の終わりまで数える）と合う．in-flight の結果が short か失敗なら，閉じる位置は受けた cursor になり，尾は捨てる（打ち切りの閉じと一つにまとめる）．受付の範囲は今の B = at_or_after(t＋W)＋3 のままで，発行した sample は封じてあり，発行の now は lock の中で読む（§5.4.3）ので，c_I ≤ B − 3 がいつも成り立つ．受付の答えは今と同じ．
- **Mock**（M4）：E3 で，Mock の Stop の切りを MR-25 の式のまま，共有の区切りで計算する．MR-25 の切りは「the later of the stop sample, the payload handed within the in-flight window …, and any same-clock payload cut retained from an earlier Stop」（`design/09-mock-radio.md:171`）で，第二項は「時刻どおりの real-time 実行役ならどこまで渡したか」の模型である（各 burst の終わりを窓の最初の sample で切り，最初の send〈32 か burst の全部〉が窓に入らない burst は渡していない）．D′ ではこの第二項を，profile の装置の値（U2）と send の大きさの上限なしで共有の区切りに問う．**Stop の sample は c_I の下限として残す**．Mock 自身の発行と block の出し方（stepped 実行役）は装置の最初の send の最小に縛られない（§5.4.3）．だから W = 0 でも MR-25 が成り立つ（再確認の列：`ideal` で A(100)@t0，Stop@t0＋10 → 切りは Stop の sample t0＋10．10 sample が出て記録の長さは 10．今と同じ）．Mock だけの模型（`tx_handed_end`）は消え，同じ区切りの関数の答えになる．UHD の c_I は send の大きさ（W/4）の刻みと host の遅れの分だけ手前になりうる（今の parity の許しと同じ）．
- **(a)（規則の cursor c_L = at_or_after(t＋W) まで送る）は後の明示の契約の変更**．本当の代価：Stop の後に，まだ発行していない payload を足して送る．実行役が止まっていれば c_L − c_I は send 一つ分（W/4）を超え，最大で W 全部になる（第 1 版の「最大で send 一つ分」は誤り）．窓の中でまだ始まっていない別の予約を始めるかを決める必要がある．stall や underflow の時には c_L に届くことを保証できない．RM-16，UR-23，MR-25，KL-13 が実質変わる．今の実測（9.28〜10.49 ms，bench-results:229-245）は今の実装の観測で，(a) の上限を確かめたものではない．

#### 5.4.9 lock の下の仕事の上限（finding 6）

- 事実（VERIFIED）：streams はふつうの `std::sync::Mutex`（`core.rs:5,786`）で，instance に一つ．uhd-control，すべての uhd-rx-s<k>，すべての uhd-tx-s<k> が取る．uhd-rx は受信の block ごとに取る（`rx.rs:532-537`）．どのスレッドも装置を呼ぶ間は持たない（`core.rs:252-256`）．`check_device` は持たない（`control.rs:616-644`）．**第 1 版の「uhd-control が装置を呼ぶ間 TX が lock を待ちうる」という読みは誤り**で，「lock を 3 回以上取っているので悪くならない」も根拠にならない（Sol finding 6）．
- 設計の条件は**実行役が lock を待つ最長の時間**で，取る回数ではない．D′ は一回の保持の仕事に上限を置く：
  - `receive`：生きている予約（R12 の prune）の上だけを調べる．波形の decode は lock の前（今も `control.rs:432-447` で decode してから `:506` で取る）．受付の now は lock の中で読む（§5.4.10）．
  - `next`：操作を**一つだけ**作る．samples は波形の `Arc` と offset（copy しない）．短い repeat の並べ（TXS の `tile_repeat`）は受付の後，lock の外で一度だけ（今は `tx.rs:329`，lock の外）．
  - 遅れからの追いつき：一回の保持で遅れた区間をまとめて作らない．一回に一つの操作を作り，発行するたびに lock を離す．遅れた SOB はそれぞれ自分の発行の時に late policy で判じる．
  - event と記録の行は遷移の出力として返し，lock の外で出す．
- 予算（INFERRED，設計の段で決める）：J(e) の閉じには allowance の 3 ms の余裕がある（`47s5-opus-rechecks.md:213`，a2 の再確認「3 ms − IDLE of lead」）．窓の補給は W − send の大きさ ≈ 7.5 ms のうちに次の操作が要る（W = 10 ms，send W/4）．だから一回の保持は µs の桁に抑え，最長の待ちを Manifest の `timing` に記録して試験と実機で見る．priority inheritance の mutex や actor は今は要らない（Sol §2 finding 6）．

#### 5.4.10 境目の等号，J(e)，最初の send，W = 0（finding 7）

- **封じの等号**：instant(k) = t＋W の時，k は封じていない（strict）．その瞬間の受付は k に予約でき，実行役は k を発行しない．今の両方の式（`tx.rs:454-455,504` と `staging.rs:566,615-617`）と同じ．第 1 版の「instant(k) − W ≤ now」は誤りで，この strict な形に直した．
- **J(e) と同じ tick の受付**：継ぎは受付の now ≤ J(e) なら受ける（inclusive，`staging.rs:559`）．「継ぐか閉じるか」は now > J(e) で封じる（今の `continuation_open` は `now <= join_by(e)` の間開いている，`staging.rs:252-254`）．32 を超える chain の最後の sample を運ぶ操作は，継ぎが入るか J(e) を過ぎるまで発行しない（今の保持の尾と同じ締め切り）．受付の now は **lock の中で読む**（今の `book_burst` は lock の前に読む，`control.rs:455` と `:506`．値の受付は lock の中で読む，`:280-281`）．DeviceAuthority の now は re-anchor をまたいでも単調（`authority.rs:143-144`，`fetch_max` は :144，VERIFIED）なので，lock の順が時刻の順になり，now ≤ J(e) の受付が「閉じた」後に来ることはない．今の `close(e)` の書き込みは要らなくなる．
- **最初の send の規則（real-time 実行役だけ，M4）**：device burst の最初の `Send` は，min(32, その burst の残り) の sample が封じてから発行し，そのとき封じた分（send の上限まで）を運ぶ．続きの `Send` は運ぶ sample がすべて封じてから発行する．今の `tx.rs:633-637`（`avail < if device_sob { wanted.min(min_open) } else { wanted }` なら送らない）と同じ（VERIFIED）．32 は装置の規則で，profile の値（U2）として区切りに渡す．MockRadio の stepped 実行役の発行には当てない（§5.4.3）．Mock の Stop の切りを計算する時だけ，MR-25 の第二項の模型としてこの規則を問う（§5.4.8）．
- **W = 0（`ideal`）**：`ideal` の timing はすべて 0（`crates/ezsdr-radio/src/channels.rs:349-352`，VERIFIED）．RM-15 は「A zero window preserves the ordinary behavior」（`design/07-radio-model.md:165`）．UHD Module は x3xx 以外の profile を拒むので（UR-5，`design/18-uhd-radio.md:135`，VERIFIED），W = 0 で回るのは MockRadio の stepped 実行役だけである．W = 0 では sample は自分の時刻を過ぎて封じられ（instant(k) < t），stepped 実行役はその step で発行し，最後の sample が過去になった時に block を出す（MR-15）．ある sample の時刻に処理した Action はその sample をまだ送っていない（MR-15 `design/09-mock-radio.md:134`）．最初の send の 32 は stepped 実行役の発行を止めない（上の項）．stepped 実行役は発行の時に遅れを判じないので，t の予約が t に受け付けられれば（lead 0，SC-27 で OnTime）時刻どおりに出る．W = 0 の Stop の切りは MR-25 の式で Stop の sample になる（第二項の窓は空，§5.4.8 の下限）．Sol の列（ideal で A@t を確定し，t より後に確定すると Drop）は起きない．real-time の実行役が W = 0 で回ることはない（先に渡せないので）．

#### 5.4.11 共有の実装を呼ばない oracle（finding 9）

区切りの判断を両 Provider が共有すると，parity は区切りの独立の検査でなくなる（設計メモ 45 §9 が「Mock と UHD の engine を一つに」を退けた理由，`plan/phase8/45-uhd-decision-core.md:272`）．FakeDevice は packet の合法さを独立に模す（短い最初の packet，gap，underflow，#70，`device.rs:641-734`）が，合法な packet で波形の位置を一つずらす誤りは通す．Mock の DeviceModel は SOB と EOB の開閉しか見ない（`crates/ezsdr-mock-radio/src/device.rs:9-38`）．

D′ は小さな **spec の oracle** を足す．`ezsdr_radio` の状態機械も区切りの関数も呼ばず，spec の文（RM-13 の波形の並び，RM-15・RM-16・RM-37 の境目）から書く（数百行，INFERRED）．入力は受け付けた予約・Stop・差し替えの列，出力は：

- 指定した装置の sample で鳴るはずの波形の sample と位相（どの予約の何番目か．repeat は (k − origin) mod N，差し替えの後は新しい周期）
- Kernel の burst の identity と payload 数
- Stop と差し替えの境目の範囲（Stop：t 以後かつ B 以前，差し替え：格子の上）
- 装置の send の trace と Kernel の burst の記録の対応

比べる相手はすでにある：FakeDevice の `transmitted(from, n)`（装置が各 sample で出したもの，channel 0，`device.rs:978-995`）と `calls()`（装置の呼び出しの列，`:956-959`），Manifest の `bursts` と `switches`．多 channel を見るには `transmitted` を広げる（INFERRED）．

役割：FakeDevice は packet の合法さと装置の trace，oracle は利用者に見える intent，実機は FakeDevice の規則の正しさ，parity は二つの実行役（stepped と real-time）の差．oracle は**移し替えの前に今のコードで通す**（§12 の O1）．今の意味を実装から独立に固め，E1〜E3 はそれを緑のまま保つ．

#### 5.4.12 代価と危険

- 大きな移し替え：tx.rs の本体（約 1 300 行，テストを除く `tx.rs:1-1309`）の判断の大半と，`book_burst`（`control.rs:392-617`），Mock の送信の道（`lib.rs:1159-1210,1577-1830`）を状態機械へ移す．段階 5 の行 185 本と，送信の N48S1・N48S3・N47S4 の行の多くを付け替える．Own の stage-5 のテスト 41 本（`tx.rs` の `ur_2[123]_(s<n>|m1|s4…)`）は大半が状態機械の純粋な試験になる．AGENTS §9 の risky（queue，deadline，寿命，fake と実機の境）．
- 操作の identity と結果の処理は残る．「報告で何も変わらない」ほど単純にはならない（Sol §4）．
- 区切りの parity の力を失い，§5.4.11 の oracle で埋める．
- 一回の lock の保持の上限と，最長の待ちの測定が要る（§5.4.9）．D′ の実装はまだなく，lock の待ち・確保・時間の上限は確かめていない（INFERRED）．
- W，lead，allowance は INFERRED のまま（U1，R5）．D′ は W の値に依らない形にしたが，W = 0 は stepped 実行役だけが回せる（§5.4.10）．

#### 5.4.13 D′ は B に勝つか

勝つ．B の意味（一人の持ち主，確定の地平，一方向の実行役）をすべて持ち，(b3) の割り込み，(b8) の controller の遅れ，(b9) の新しい値がない．D′ は B の一つの実装形である．

### 5.5 E：契約を狭めて記録を小さくする（X4「順番どおりだけ」が中心）

予約は開始の順にだけ受け付ける：受け付けたどの予約より前に始まる予約も拒む（KL-37 を W に限らず広げる）．時刻なしは「最後の予約の後」（queue）．すると記録は数個の値になり，`place` の繰り返し，`first_free`，`held_until`，`may_be_handed`，`met_front`，`region_move`（`staging.rs:268-311,537-579`）が要らなくなる．

- 失うもの：後の予約を先に受け付けた後で，前の予約を足すこと．**固定の TDD の slot を先に予約した後に，その前の空きへ反応の ACK・ARQ を入れること**（Sol §5．Reactor の受付の順は target の順を保証しない）．遠い未来の予約の前の隙間に時刻なしの send を置くこと．
- 第 1 版は「Vision §22 は時刻の順に答えるので，ほぼ失わない」（INFERRED）と書いたが，上の TDD の列はそれを否定する．Vision §22 は「packet responses, ARQ, TDD, reactive protocols」を必須と書く（`design/vision/04-execution-model-and-data-contracts.md:227-233`）．
- **退ける（§11.6）．** 区間の論理は残るが，D′ では状態機械の一か所にある．

### 5.6 F：装置だけが予測する（記録を持たず，装置の `TIME_ERROR` に反応する）

UHD・SoapySDR・GNU Radio はこの形である（§5.11）．受付ではほぼ何も拒まず，開始の順に渡し，装置の遅れの報告で知る．

- 原則 4（受け付けた予約との衝突は拒否）と O2（submit の返事の `Rejected`）を開き直すことになる．
- Mock は装置の FIFO と Own の渡す時刻を模すことになり，結局は予測を持つ．しかもその時刻は USRP では host に依るので，Mock と同じ答えにならない（Vision §59）．
- 先例がこの形でよいのは，受付の時点の約束がないからである．
- **退ける．**

### 5.7 G：決める瞬間を確定の時へ動かす（受付は静的な検査だけ，重なりは確定の時に非同期で拒む）

- 確定の時にも，すでに確定したものの区間を知る必要がある．同じ区間の論理を後でするだけになる．
- O2 の `Rejected` を失う．
- **退ける．**

### 5.8 H：stream ごとに一本のスレッド（actor）

uhd-control が送信の Action をその stream のスレッドへ渡して返事を待ち，そのスレッドが予約と送信の両方を持つ．

- スレッドの間の窓（S1，M1(c)）は消える．計画と実際の二つの写しは，一つのスレッドの中に残る．
- send は長く止まりうる：`SEND_TIMEOUT`（1 s，`tx.rs:235`）は UHD の中で fragment ごとに同じ値で渡され，全体の上限ではない（UHD 4.10 の [tx_streamer_impl.hpp](https://raw.githubusercontent.com/EttusResearch/uhd/v4.10.0.0/host/lib/include/uhdlib/transport/tx_streamer_impl.hpp)：`timeout_ms` を一度だけ計算し，各 fragment の `_send_one_packet` に同じ値で渡す，VERIFIED）．20 000 sample の send は spp 約 2 000（bench-results:155）で約 10 fragment なので，最悪で約 10 s 止まりうる（INFERRED：算術）．その間，受付の返事が遅れ，Kernel は uhd-control を待つ（KC-21a）．**第 1 版の「最大 1 s」は誤り．**
- スレッドの形は ④ の問い．D′ はこの形に載せ替えられる．**今は採らない．**

### 5.9 I：0 で埋めて継ぐ（srsRAN の形）

radio が小さな隙間を 0 で埋めて device burst を開いたままにし，大きければ閉じる（srsRAN 4G の `radio.h`：「Maximum transmission gap to fill with zeros, otherwise the burst shall be stopped」，VERIFIED）．

- 小さな隙間なら閉じ方の front（+1・+2・+5）が要らなくなる．
- 継ぐか閉じるかは，やはり発行の前に決める必要がある．
- 隙間の間も送信が鍵を開けたまま（ATR が送信の状態）になり，同じ board の受信への影響は INFERRED．
- Kernel の burst の中の 0 埋めは Stream Contract が禁じる（§58 の 15）．burst の間なら許されるが，記録の約束が一つ増える．
- **今は採らない．** Phase 10 の TDD が front より短い隙間を要する時に，D′ の操作の種類として足せる．

### 5.10 J：一本だけ（stream に予約は高々一つ）

値の規則と同じく，空の stream にだけ予約を受け付ける．記録はほぼ消える．しかし Reactor の先読み（次の slot を今の slot の間に予約すること）ができず，Vision §22 の反応する送信が成り立たない．**退ける．**

### 5.11 外の先例

| システム | 可変の予定 | 確定の時点 | 実行役 | 受付（enqueue）での拒否 | 遅れの扱い | 当てはまるか |
|---|---|---|---|---|---|---|
| UHD の送信（`tx_metadata_t`，`async_metadata_t`，[metadata.hpp](https://raw.githubusercontent.com/EttusResearch/uhd/v4.10.0.0/host/include/uhd/types/metadata.hpp)） | なし（host が決める） | `send` | 装置の FIFO．`time_spec`「When to send the first sample」，SOB・EOB | なし | 非同期の `EVENT_CODE_TIME_ERROR`「Packet had time that was late」，BURST_ACK，UNDERFLOW．報告は channel・時刻・code・user payload だけ（VERIFIED） | D′ の実行役そのもの |
| UHD の時刻つき命令（`set_command_time`，`design/18-uhd-radio.md` §2） | なし | 発行 | FPGA の queue．後の命令を back-pressure | なし | 遅れた命令は着いた時に実行（VERIFIED） | 同上 |
| GNU Radio `usrp_sink`（[usrp_sink.h](https://raw.githubusercontent.com/gnuradio/gnuradio/main/gr-uhd/include/gnuradio/uhd/usrp_sink.h)） | flowgraph（作る側） | stream の順 | sink → UHD．`tx_sob`・`tx_eob`・`tx_time`・`tx_command` の tag を消費 | なし | 遅れや重なりについて文書は何も言わない（VERIFIED） | 続けるかは作る側の仕事 |
| SoapySDR（[Device.hpp](https://raw.githubusercontent.com/pothosware/SoapySDR/master/include/SoapySDR/Device.hpp)） | なし | `writeStream`（flags，timeNs） | 装置 | なし | `readStreamStatus`「report time errors, underflows, and burst completion」（VERIFIED） | D′ の実行役 |
| srsRAN 4G（[common.h](https://raw.githubusercontent.com/srsran/srsRAN_4G/master/lib/include/srsran/common/common.h)，[radio.h](https://raw.githubusercontent.com/srsran/srsRAN_4G/master/lib/include/srsran/radio/radio.h)） | MAC の scheduler（TTI ごと） | 固定の lead：`TTI_TX(tti) = TTI_ADD(tti, FDD_HARQ_DELAY_DL_MS)`，4 TTI 先（VERIFIED） | radio のスレッド `tx(buffer, tx_time)` | — | 小さな隙間は 0 で埋め，大きければ burst を止める（VERIFIED） | B の controller と固定の地平．I の 0 埋め |
| Linux ETF qdisc（[sch_etf.c](https://raw.githubusercontent.com/torvalds/linux/master/net/sched/sch_etf.c)） | txtime で並べた列．順不同に入れてよい | 「Dequeue only if now is within the [txtime - delta, txtime] range」 | NIC の launch time（FIFO） | `ktime_before(txtime, now) \|\| ktime_before(txtime, q->last)` なら `SO_EE_CODE_TXTIME_INVALID_PARAM` で落とす．`q->last` は最後に NIC へ渡した txtime（VERIFIED） | 列の中で期限が過ぎたら `SO_EE_CODE_TXTIME_MISSED` で落とす（VERIFIED） | **B・D′ とほぼ同じ形．** delta が W，`q->last` の拒否が KL-37 と同じ規則 |
| Linux taprio（[sch_taprio.c v5.2-rc2](https://android-kvm.googlesource.com/linux/+/refs/tags/v5.2-rc2/net/sched/sch_taprio.c)，[tc-taprio(8)](https://man7.org/linux/man-pages/man8/tc-taprio.8.html)） | admin の schedule（変えてよい） | cycle の終わりで oper へ昇格（`should_change_schedules`，`switch_schedules`） | oper（走っている間は読むだけ） | — | base-time が過去なら「base-time + (N * cycle-time)」，N は今より後になる最小の整数（VERIFIED） | **差し替えの境目の規則（RM-37）と同じ** |
| RFNoC Replay（[replay_block_control.hpp](https://raw.githubusercontent.com/EttusResearch/uhd/v4.10.0.0/host/include/uhd/rfnoc/replay_block_control.hpp)） | host | `play` | 装置の命令の列．「Commands are queued and executed in order」．`stop()` は「cancels any other play commands that are waiting」 | 「Too many play commands are queued」 | 時刻は最初の packet の header に入るだけで，block は待たない（VERIFIED） | 8c の実行役の一つの形 |

読めること：

- どの先例も，可変の予定（ETF の並べた列，taprio の admin，srsRAN の scheduler）と，一方向で順番どおりの実行役（NIC，oper，radio のスレッド，UHD，Replay）を分け，固定の lead（delta，4 TTI，cycle の境目）で確定する．B・D′ と同じ．
- 受付の拒否は「過去」か「確定したものより前」だけ（ETF）．KL-37 は業界の普通の規則である．
- どの先例も，確定した後の実行役の状態を受付で予測しない．実行の結果は非同期の報告で返る（UHD，SoapySDR，ETF の error queue）．D′ はこの報告を名乗った操作・device burst に当てる．
- どの先例も，別々に予約したものを確定の後で一つの burst へ継がない．続けるのは作る側（GNU Radio）か，radio の 0 埋め（srsRAN）の仕事．継ぎ（F7 (b)）は Ez-SDR だけの機能で，その代価が R4・R5・U3 である．

### 5.12 設計メモ 45 との関係

45 がもう答えたこと：

- Module 全体を一つの判断役のスレッドに集め DST で探る形は，前もって決めた条件を二回続けて満たさなかった（`plan/phase8/45-report.md:8-11`，`45-report-e4.md:8-13`）．DST の力は模型の範囲と粒度で決まる（`45-report-e4.md:79-81`）．**D′ はこれをしない**：新しいスレッドも，cell と受け入れの手順も，DST の実験も持たない．範囲は送信の予約だけで，試験は純粋な遷移の表，生成した操作の列（`rm_26_generated_sequences_keep_the_timeline_invariants` と同じやり方），§5.4.11 の oracle．
- 一人が持つ形は，過去の不具合 10 件のうち 4 件を起こりえなくした（`45-report.md:85-89`，`45-report-e4.md:63`）．D′ が頼るのはこの性質である．
- 45 §4.4 の「命令の状態」（列にある，取った，効いたかもしれない，終わった，始末した，`45-uhd-decision-core.md:102-114`）は，D′ の発行・結果の区別と同じ問題への答えだった．D′ はそれを送信の一つの stream と in-flight 一つに絞って使う．
- owner は §9 の第一の代わり（判断を純粋な関数として今のスレッドの中に取り出す）に移った（`45-report-e4.md:79-92,102`）．段階 5 は送信の判断をこの形で取り出したが，**状態は二つ（記録と Own の写し）のまま**残した．D′ は状態も一つにして，その代わりを最後までやる．
- stream ごとの actor は `recv` で止まるので二度退けた（`45-uhd-decision-core.md:63-65`）．送信には `recv` がなく，D′ はスレッドを足さないので当たらない．

### 5.13 ④（Module の形）との関係

D′ が決めるのは送信の状態の持ち主と，実行役との protocol で，スレッドの形は決めない．D′ は今のスレッド（uhd-control，uhd-tx-s<k>）のままで動く．④ が後で送信の受付を stream のスレッドへ移すなどしても，状態機械はそのまま移る．④ の比較の入力は「状態の持ち主は D′ で一つ」とする．

### 5.14 結論

| 案 | 同じ intent の写し | 利用者の損 | 代価 | 判定 |
|---|---|---|---|---|
| A | 残る．道が増えるたびに書き戻しが増える | なし | 機構が残る | 戻り先 |
| A′ | P3 の一部が消える．P1・P2・P5・P6 は残る | X1・X2 の分 | 小さい | 退ける（X1 は N0 で採る） |
| B（文字どおり） | 消える | 長い burst の Stop が遅れる（割り込まないなら） | controller のスレッド，その遅れ，地平の値 | 意味は採り，形は D′ |
| C | 記録の側を消すと parity と O2 が崩れる | O2 | — | 退ける |
| **D′（決定）** | **消える**．予定と実行の違いは一つの持ち主の中で明示 | **最初の移行はなし**．契約の変更は §11 で一つずつ | **大きな移し替え．操作と結果の protocol．区切りの parity の力（oracle で埋める）** | **決定（owner 2026-11-08）** |
| E（X4） | 残る | TDD の slot の前の反応の送信（Vision §22） | 小さい | 退ける |
| F | 消える（予測しない） | 原則 4，O2 | Mock が host の時刻を模す | 退ける |
| G | 残る | O2 | — | 退ける |
| H | 残る（一つのスレッドの中） | 受付の遅れ | スレッドの形 | ④ で |
| I | 残る | 送信が鍵を開けたまま | — | 今は採らない |
| J | ほぼ消える | 先読みの予約（Vision §22） | — | 退ける |

## 6. 判断の原則（場合を決める文）

1. **一つの事実に書き手は一人．** 予約・区切り・差し替え・Stop・実行の結果は状態機械，装置の事実（streamer，報告の照合）は実行役．
2. **確定した intent を実際の実行と読まない．** 封じた，発行した，送った，鳴ったを別の状態として持つ（§5.4.2）．受付は「封じた」だけを読み，記録と outcome は「送った」「鳴った」から書く．
3. **受付・発行・結果の判断は (状態，入力，装置の時刻) の純粋な関数．** Mock と Module に同じ値を与えれば同じ答えを出す．real-time の実行役だけが発行の時に遅れを判じる（今の dispatch と同じ）．
4. **原則 4 の当てはめ**（§11.2）：生きている予約の footprint（payload と，その予約自身の装置の閉じ方）に当たる予約は**拒否**．それ以外で開始に間に合わないもの（Stop の drain，host の stall，打ち切りや recovery の残り，lead）は **late policy**．時刻なしは最初の空き．KL-36 は題名が「decided late」，本文が「refused at receipt」で食い違っている（`KNOWN_LIMITATIONS.md:134-136`，VERIFIED）．この文で本文の側にそろえる．
5. **結果と報告は，名乗った操作と照合した device burst にだけ当たる．** 一度だけ当て，結果を当ててから次を発行する．後の burst の予約は変えない．Stop は in-flight の操作の結果を当てた後で，まだ発行していないものを取り消す（今の順，§5.4.4 の 4）．打ち切りの後の記録の範囲は今のまま：有限の予約は範囲を残し，repeat はその op の burst の分だけ cursor で終える．範囲を縮めるのは別の契約の変更（§5.4.4 の 2）．
6. **装置の規則（32，4，2，+1）は profile の値．** 状態機械は値として受け取り，real-time 実行役の区切りにだけ当てる．Mock の stepped 実行役の発行は縛らず，Mock は Stop の切りの模型（MR-25 の第二項）にだけ使う（§5.4.3，§5.4.8）．測った値（28 と 30，1・2 MS/s の 4）と安全側の値（32，すべての rate の 4）を分けて書く（`device.rs:651-663`）．今は X3x0 の値をどの profile にも当てている（計画の再確認 m7，`47s5-test-plan.md:738`）．
7. **lock の下の仕事には上限がある．** 一回の保持で操作は一つ．decode・copy・event の発行は外．条件は実行役の最長の待ち．
8. **時刻なしは Kernel の性質**（原則 1）．radio では最初の空き．
9. **契約を狭めるのは，Easy API と Vision §22 の使い方が失わない時だけ．** 失うなら機能を残し，その代価を表に書く．
10. **抽出と契約の変更を分ける．** 抽出は今の振る舞いを変えない．契約の変更は両 Provider を同時に変え，一つずつ owner が決める．

## 7. 既存の約束との照合（D′ の最初の移行と，§11 の決定の契約の変更）

| 約束 | 最初の移行（N0，O1，E1〜E3） | 契約の変更（決定） |
|---|---|---|
| EA-16 の `repeat`・`switch`・`stop`（`design/16-easy-api.md:85`） | 同じ | `switch` の outcome（C2） |
| EA-16 の値・rate | 同じ | — （送信の rate は後の独立した段階，§11.5） |
| RM-12，RM-13 | N0 の X1：switch の印は SendAsapAndFlag の burst にだけ（ほかは RM-13 で拒否） | — |
| RM-14 | 文だけ：「dispatch で判定し直す」を「発行の時に判定する」へ．意味は同じ | — |
| RM-15 | 同じ．§6 の 4 の文を足す（N0．振る舞いは変えない） | C1：KL-38 の例外を消す（拒まれた予約の後ろは区切り直す） |
| RM-16 | 同じ（発行した所で閉じる．保持した尾は閉じの send で出る，§5.4.8） | Stop (a) は今は採らない（§11.8．C4 は owner が後で承認した時だけ） |
| RM-21 | 同じ | — |
| RM-37 | 規則も文も同じ．「the owner moves it only under its late policy when its cursor has passed it」（`design/07-radio-model.md:346`）の移動は，E2a で UHD も受付で取り込むので起きなくなるが，文は偽にならない（INFERRED，§5.4.5） | C2：outcome を境目を送ったかで決める（`uncertain` を足す） |
| UR-21，UR-22，UR-23，UR-36，UR-38 | 実行役と操作として書き直す（意味は同じ）．UR-22 の装置の規則と打ち切りの後の記録の範囲はそのまま．UR-38 の取り込みは受付の時に発行した cursor で（E2a，§5.4.5） | C1，C2 |
| UR-25，UR-26 | configure の turn は `Configure` の操作に（順序は同じ） | C1 |
| UR-28 | 同じ（照合は今と同じ材料） | — |
| MR-16，MR-17 | 同じ | — |
| MR-25 | E3：第二項（渡した payload の模型）を共有の区切りに profile の装置の値で問い，Stop の sample を下限に残す．Mock の発行は装置の最初の send の最小に縛られない．振る舞いは同じ（W = 0 でも，§5.4.8） | — |
| MR-42 | 同じ（Mock は今も受付で取り込む．E2a で UHD も受付で取り込む，§5.4.5） | C2：Stop の行の outcome と，取り込みと outcome の文を改める |
| KL-13，KL-15，KL-37 | 同じ | — |
| KL-36 | 題名を本文にそろえる（N0） | §11.7 の決定：(b) のまま．C5 は要る時に後で |
| KL-38 | 同じ | C1 で撤回 |
| Vision §58 の 11，15，§61 | 同じ | 同じ |
| AGENTS §3 | 状態機械は radio の Vocabulary の crate の中．Kernel は変えない．置き場所は最初の空き（最適化しない） | 同じ |

## 8. 実機の証拠のまとめ

| 事実 | 印 | 出典 |
|---|---|---|
| 別の device burst の SOB が前の burst の終わりと同じ tick なら捨てられる（gap 0）．gap 1 以上は鳴る（EOB が最後の data buffer にある時） | VERIFIED | bench-results:100,128,243（三つの記録と shake-out） |
| 空の EOB（UHD の 0 一つ）で閉じると gap 0・1 が late，2 以上は鳴る | VERIFIED | 同上 |
| 保持の尾の閉じ（尾＋0）の後は end＋1 が late，end＋2 は鳴る | VERIFIED | 2026-11-06，二台，bench-results:369 |
| 閉じる最初の send は 1・2 MS/s で 4 sample 未満が落ちる | VERIFIED（その rate で） | 2026-11-05 C++，bench-results:322-332 |
| 開いたままの最初の send は，補間 80 以上で 28 以下なら Underflow，30 以上なら問題なし．29 は未測定 | VERIFIED（その条件で） | 2026-11-06，bench-results:345-353，`device.rs:657-663` |
| 32 と 4 をすべての rate と profile に当てること | INFERRED（安全側の選択） | `device.rs:651-663`，計画の再確認 m7 |
| 閉じた後の安全な開始は閉じの終わり＋1 | VERIFIED | 2026-11-05 C++，bench-results:333 |
| 一つの device burst に続ける継ぎ | VERIFIED | `hw_b8_burst_at_a_sent_burst_s_end`，2026-11-06，bench-results:342,369 |
| Stop の提出から音が止むまで 9.3〜10.5 ms（今の実装） | VERIFIED（今の実装の観測．Stop (a) の上限ではない） | bench-results:50,229-245 |
| underflow の後の遅れた続きは送られない | VERIFIED | P6 starvation，bench-results:223 |
| 前の burst の ACK の後の時刻なしの送信の設定は末尾を乱さない | VERIFIED | T5，bench-results:311 |
| Module を通る最小の lead は 3 ms | VERIFIED | bench-results:49 |
| UHD の `send` の timeout は fragment ごとで，全体の上限ではない | VERIFIED（source） | UHD 4.10 `tx_streamer_impl.hpp`（§5.8） |
| UHD の非同期の報告は予約の id を持たない | VERIFIED（source） | `metadata.hpp` の `async_metadata_t` |
| 予約の境をまたぐ最初の send（短い頭＋継ぎ）を装置は一つの send として扱う | INFERRED | 装置は packet を見て予約を見ない．§11.7 (a)・C1 の前提 |
| 閉じた頭（1〜32，1〜3 は 4 に詰める）の後の装置の前線は送った終わり＋1（gap 0 は鳴らず，gap 1〜5 は鳴る） | **VERIFIED**（2026-11-09 (c)2，二台，1・2 MS/s） | bench-results の「2026-11-09」．ただし Module を通すと前線の burst が飢える（(c)1 FAIL，Module の送り方の欠陥，issue #81） |
| W = 10 ms，device lead 2 ms，delivery allowance 3 ms | INFERRED | spec 18 §3，KL-15．W の測る群は段階 5 で消えた．Stop を使う測り方は未作成 |
| Stop の後の +3（空の閉じの後の c＋3，c＋4） | **VERIFIED**（2026-11-09 (d)，二台） | bench-results の「2026-11-09」．**1〜3 sample の頭を c − 1 に詰めた形は記録が誤り**：送った終わりは c＋3 で，c＋3 の予約は gap 0（二台とも FAIL）．正しくは c＋4（issue #82） |
| J(e) の前の継ぎ，J(e) での閉じ，e＋1 の拒否 | **VERIFIED**（2026-11-09 (d)，二台） | bench-results の「2026-11-09」 |
| Session の Stop の後の最初の sample の burst | INCONCLUSIVE（2026-11-09 (d)，host latency が記録にない） | bench-results の「2026-11-09」 |
| 差し替えの境目（applied，継ぎ目なし） | **VERIFIED は #1 の時刻つきの 1 行**（tone 100→200 kHz，電力は保つ，継ぎ目なし）．ほかの 3 行は試験の判定の欠陥で FAIL（capture 全体の窓を見る，INFERRED） | bench-results の「2026-11-09」 |
| 時刻なしの `repeat; stop; repeat` と，流れている burst の後ろの時刻なし | **VERIFIED**（2026-11-09 (f)，二台） | bench-results の「2026-11-09」 |
| 段階 5 の記録での Stop の終わり | **VERIFIED**（2026-11-09 (g)，二台．記録した終わりから −22〜56 µs） | bench-results の「2026-11-09」 |
| 開いたままの streamer での送信の rate の変更 | **VERIFIED**（2026-11-09 (h)，二台．11-06 の #2 INCONCLUSIVE は消えた） | bench-results の「2026-11-09」．§11.5 の後の段階の材料 |
| 送信中の ssh 切断で送信が止まる | **VERIFIED**（2026-11-09 (i)，#1 の送信を #2 で受けた．C1〜C3 PASS） | bench-results の「2026-11-09」 |
| 回帰（S1・P70・P72・B6〜B8・H5・TXP） | **VERIFIED**（2026-11-09 (b)，二台 PASS） | bench-results の「2026-11-09」 |

D′ の最初の移行は区切りの値を変えないので，今の設計と同じ証拠に頼る：上の VERIFIED の行，INFERRED の行（W，lead，allowance，4〜32 の +1，空の閉じの後の +3，32 と 4 の安全側の値），2026-11-09 に確かめた行（(c)2，(d)，(g)．Stop の後の詰めた頭の境は記録の誤りで #82）．**第 1 版の「D が頼る装置の事実はすべて VERIFIED」は誤りで，この文に直した．** 契約の変更 C1 と §11.7 (a) は，INFERRED の「予約の境をまたぐ最初の send」にも頼るので，実機で一度確かめてから入れる．

## 9. この選択で後からできなくなること

- **8b（複数台）**：状態機械は stream ごとで，揃った開始は各 stream の同じ `at` の予約で表す．台をまたぐ継ぎはできない（別の stream なので今もできない）．妨げはない．
- **8c（RFNoC Replay）**：Replay は自分が一方向の命令の列で，`stop()` が待つ命令を消す（§5.11，VERIFIED）．D′ の操作が「play の命令」になる形で載る．repeat は連続の play．差し替えは「境目までの回数の play」と次の play で作れる見込み（INFERRED）．操作と結果の protocol は命令の列にそのまま合う．妨げはない．
- **ほかの SDR**（`plan/maintenance/33-radio-generality.md`）：bladeRF は FPGA に遅れて着いた sample を報告なしにすぐ送る（33 の G3，`33-radio-generality.md:85`）．D′ の発行の時の late policy が唯一の守りになり，今の形より合う．HackRF は時刻つきの送信がない（K2）．時刻なしの実行役で表す．区切りの値を profile に置く（§6 の 6）ことが前提．
- **Phase 10**：link から送る stream は，状態機械の発行が link の buffer から sample を引く形で表せる．Reactor の反応する burst（Vision §22）は短い lead の予約で，J(e) = e − 5 ms の継ぎと，順番の逆の予約（X4 を退ける）が要る．
- **D′ でできなくなること**：発行した操作を変えること（物理的に今もできない）．Provider ごとに予約と区切りの判断を変えること（設計で禁じる）．Mock と Module の区切りの食い違いを parity で見つけること（§5.4.11 の oracle，FakeDevice，実機が見る）．

## 10. 機能を削る候補（owner 2026-11-08 にすべて決めた）

| # | 候補 | 使う場面 | 今だれが使うか | 消すと簡単になるもの | 決定 |
|---|---|---|---|---|---|
| X1 | DropAndFlag の差し替え（境目を過ぎたら拒む枝） | 遅れたら動かずに諦める差し替え | なし．`start_repeat` は SendAsapAndFlag に固定（RM-12）で，Python の `switch` もそれを送る（`session.py:504-512`）．届くのは将来の Module の burst だけ | `install` の拒否の枝（`staging.rs:396-398`），結果 `refused_past_boundary`，UR-38 の Drop の文，行 N47S5-42/76，`ur_21_s12_drop_and_flag_switch_handed_past_is_refused` | **消す**（N0，§11.3）．印は SendAsapAndFlag の burst にだけ．ほかは RM-13 で拒否．将来の Reactor の「期限に敏感な差し替え」を失うことを明記する（Sol） |
| X2 | 値を変えた直後の burst への継ぎ | 周波数を変えた直後に隙間なく続ける | raw の `send` と将来の Reactor | KL-38 の一部．ただし「最初の burst」だけでは，同じ generation の retry（configure の拒否の後の次の burst）と recovery を覆わず，KL-38 は残る（Sol finding 4） | **採らない**．C1（configure を payload の前に終え，発行していない piece を区切り直す）で KL-38 ごと消す（§11.4 の決定） |
| X3 | 送信の rate の変更（F2 ① の rate の部分．まだつくっていない） | Run の中で送信の rate を変える | なし（EA-16 は `Error`．bench (h) の受け入れの条件も未決） | 値を伴う burst で新しい送信の clock を登録する未決の設計（configure を拒まれた clock，再試行の道．`47-no-change-while-streaming.md:122-123`），「stream ごとに clock は一つ」が Run の不変条件のまま | **採らない**（送信の rate の変更は消さない，§11.5）．2026-11-04 の決定（F2 ①・F4b：送信の rate は空いた stream に予約した burst の parameter）を残し，D′ の後の独立した risky の段階として手順 0 から作る．第 2 版の推奨（つくらない）は採らなかった．代わりが同等でないこと（新しい Session は T0・ほかの stream・Run を共有せず，resampling は装置の sample rate の変更と同じではない，Sol）も理由の一つ |
| X4 | 順番の逆の予約と，隙間への時刻なしの置き場所 | 後の予約の前に予約を足す．TDD の slot の前に反応の ACK・ARQ を入れる．遠い未来の予約の前の隙間に今すぐの send を置く | raw と将来の Reactor | R2・R7・R10・R12 が比較数個に（`staging.rs:268-311,537-579`） | **残す**（X4 は採らない，§11.6）．予約は開始時刻を key にした ordered map で持つ（owner の注） |
| X5 | 遅い継ぎ（保持の尾で J(e) = e − 5 ms まで待つ） | 前の burst の終わりの 5〜10 ms 前に届く継ぎ | raw と将来の Reactor | `Tail`，`hold`（`tx.rs:655-662`），`close_unjoined`，`continuation_open`・`close`，+2 の front が +1 に，H5 の期限の未決，KL-13 (d) | **残す**（§11.9）．消すと継ぎの締め切りが普通の時刻つきの開始（5 ms）より厳しくなる．Vision §22 は反応する送信を求めるが，5 ms という値までは求めない（Sol）．Phase 10 の要求が分かった時に見直す |
| X6 | 継ぎそのもの（F7 (b)） | 別々に予約した burst を隙間なく続ける | raw と将来の Reactor | R4，R5，U3，chain と継ぎ点，KL-36 の場合分け | **残す**（§11.9）．F7 (b) は owner が望んだ（`46-streaming-feature-ledger.md:226`）．Vision §22 の TDD に要る |

## 11. owner の決定（2026-11-08）

**すべて決まった（owner 2026-11-08）．** 各節の「決定」の行が owner の答えで，その下の推奨と案は決めた時の材料として残す．行き先：N0 に §11.3（X1）と §11.2 の文（KL-36 の題名も）．E1〜E3 の形に §11.6（開始時刻の ordered map），§11.7 (b)，§11.8 (b)．移行の後の契約の変更に §11.4（C1）と §11.10（C2）．§11.5 は D′ の後の独立した段階（X3 は採らない）．§11.9 は今のまま（X5・X6 を残す）．C4（Stop (a)）と C5（短い頭への継ぎ (a)）は owner が後で承認した時だけ．

### 11.1 方向

**決定（owner 2026-11-08）：案 D．** Sol の方向レビュー（SOUND WITH CHANGES，`~/.cache/ezsdr-fixes/2026-11-08-n49/sol-direction.md`）の後，**D′ に改めた**（owner 2026-11-08）：一つの状態機械が予約・区切り・Stop・差し替えと実行の結果を持ち，executor は token 付きの操作を一つずつ受けて結果を返す．configure と recovery は payload の発行の前提．時刻による受付の封じ・segment の発行・実行の結果を別の概念にする．最初の移行では Stop は実際に発行した所で閉じる．

退けた案：A′（S1 系と S4 系の書き戻しが残る），B を文字どおり（別の controller のスレッドの遅れ，割り込み，新しい地平の値），actor（blocking な send が受付を止める）．設計の中身は §5.4．

### 11.2 原則 4 の当てはめ：予約自身の装置の閉じ方は「その予約の footprint」か「装置の状態」か

**決定（owner 2026-11-08）：(a) footprint（拒否）．** 打ち切りや recovery の後に残るものは late policy（Sol の注意）．N0 で RM-15 に文を足し，KL-36 の題名を本文にそろえる．owner の問い「burst のマージを将来入れればなくなるか」への答え：縮むが消えない（閉じ方が決まった後に届く予約，0 埋めなしの隙間 1〜2，INFERRED）．

- **(a) footprint（推奨．Sol も支持）**：生きている予約自身の閉じ方に当たる予約は拒否（今のコードと #79 の決定）．Stop の drain・host の stall は late policy（F36 と同じ）．**Sol の注意：打ち切りや recovery の後に残るもの（閉じの front など）は，生きている予約との衝突として扱わない**（late policy）．KL-36 の題名を本文にそろえる．利点：今の振る舞いを変えない．受付で答えが決まる．欠点：同じ「装置の都合」に二つの扱いがある（生きている予約の閉じ方か，それ以外か）．
- (b) 装置の状態：閉じ方に当たる予約も late policy（SendAsap は 1〜4 sample 動いて `TIME_ERROR`，Drop は `TIME_ERROR{drop}`）．利点：原則 4 の字面と D1 の「late で判定」に合う．欠点：#79 の決定を開き直す．利用者は `Rejected` の代わりに非同期の event を受ける．
- 退けた案：何も書かない（KL-36 の食い違いが残る）．

### 11.3 X1：DropAndFlag の差し替えをなくすか

**決定（owner 2026-11-08）：なくす（N0）．** 差し替えの印は SendAsapAndFlag の burst にだけ．ほかは RM-13 で拒否．

推奨：なくす（Sol も支持）．利点：今の利用者に損はない（`session.py:504-512` と RM-12 は SendAsapAndFlag に固定）．移す前に消すので，D′ に移すコードが減る．欠点：将来の Reactor が「期限を過ぎたら諦める差し替え」を作れない．その時は late policy の枝を一つ足して戻す．

### 11.4 KL-38：X2 か，configure を payload の前に終える（C1）か

**決定（owner 2026-11-08）：C1．** 移行の後の契約の変更として入れる（予約の境をまたぐ最初の send を FakeDevice と実機で確かめてから）．それまで KL-38 は KNOWN_LIMITATIONS に残る．

- **C1（推奨）**：configure は payload の発行の前提（§5.4.6）なので，拒否の時点で後ろの piece は一つも発行されていない．状態機械は拒まれた予約だけを外し，残りの piece を一つの chain として区切り直す．同じ generation の retry も recovery もこの道を通る．利点：受付の拒否を一つも増やさずに KL-38 を撤回できる．S2 系が起きない．Mock は configure を拒まないので parity は変わらない．欠点：E2b の後でないと入れられない．区切り直した chain の最初の send が予約の境をまたぐことは INFERRED（実機で一度確かめる）．
- X2′（受付で判断できる形）：「generation の configure の成功がまだ確かめられていない burst には，継ぎを受けない」．retry と recovery（印が消えた後）も覆う．利点：D′ を待たずに入れられる．欠点：多くの継ぎが拒否になる（configure は burst の発行の時なので，継ぎの受付の時点ではたいてい未確認）．受付の答えが報告された事実（印）に依り，Mock と UHD の間に host の時刻の窓ができる．
- 退けた案：第 1 版の X2（「値を変えた直後の最初の burst」だけ）．Sol の列（A が拒まれ，B も拒まれる）で KL-38 が残る．

### 11.5 X3：送信の rate の変更をつくらないか

**決定（owner 2026-11-08）：X3 は採らない．2026-11-04 の決定（F2 ①・F4b：送信の rate は空いた stream に予約した burst の parameter，受信の rate は `start_rx` の parameter）を残す．** X3 は 11-04 の決定を覆す案で，まだ作っていない機能なので D′ を簡単にしない．送信の rate の変更は D′ の移行に入れず，後の独立した risky の段階として手順 0 から作る．「stream ごとに clock は一つ」は段階 4 の簡略化（R1）で Vision の不変条件ではない（Vision §15：rate の変更は新しい SampleClock．Kernel の `running_clock`，`crates/ezsdr-kernel/src/coordinator/state.rs:336-339`．受信は `start_rx` ごとに clock を始める）．その段階で設計すること：設定を拒まれた新しい clock が origin で終わると生きた送信の clock がなくなり，直した burst が admission で拒まれる道（`admission.rs:155-157`）．Provider の側で済む見込み（INFERRED）．D′ の状態機械には generation ごとの clock が足される．受信の rate の拒否を外すのは別の小さな段階で owner に諮る．以下の推奨は第 2 版のもの（採らなかった）．

推奨：つくらない（Sol も支持）．利点：未決の clock の設計（`47-no-change-while-streaming.md:122-123`）が要らない．bench (h) の条件を閉じる必要もなくなる．欠点：F2 ① の rate の部分を取り下げる．**代わりは同等ではない**：新しい Session は T0・ほかの stream・Run を共有する実験の代わりにならず，信号処理の resampling は装置の sample rate の変更と同じではない．受信の rate は別に決める（今は両方向とも `Error`．受信の bench の条件は 2026-11-05 に満たした，`47-no-change-while-streaming.md:155`）．代わりの案：(h) が通ったら設計する（今の予定）．

### 11.6 X4：予約を開始の順にだけ受け付けるか

**決定（owner 2026-11-08）：X4 は採らない（順番の逆の予約を残す）．** owner の注：予約の列は単純な queue でなく，開始時刻で並べた木（ordered map）でよい．受け付けた予約の footprint は重ならない（R2 で拒む）ので，区間の木は要らず，開始時刻を key にした `BTreeMap` の前後の一つずつを見れば重なりを判定できる（orchestrator の補足，INFERRED：設計の段で確かめる）．

- **推奨：受け付けない（X4 を退ける）．Sol の反対を採る．** 固定の TDD の slot を先に予約した後に，その前の空きへ反応の ACK・ARQ の送信を入れるのは自然な使い方で，Reactor の受付の順は target の順を保証しない．Vision §22 はこれらを必須と書く（`design/vision/04-execution-model-and-data-contracts.md:223-233`）．第 1 版の「Vision §22 は時刻の順に答える」（INFERRED）はこの列で否定される．
- X4 の利点（もし採るなら）：状態機械の予約の列が queue になり，R2・R7・R10・R12 が比較数個になる．欠点：上の使い方を失い，時刻なしの send が遠い未来の予約の後ろに並ぶ．
- 退けたことの代価：区間の論理（重なり，隙間への置き場所，KL-37）は残る．ただし D′ では状態機械の一か所にあり，§5.4.11 の oracle が独立に固める．

### 11.7 短い頭への継ぎを確定の時刻で決めるか

**決定（owner 2026-11-08）：(b)，今の 32 sample の閾値のまま．** 32 sample 以下の送信の直後に次を送る場面は稀なので，KNOWN_LIMITATIONS の KL-36 に書いておくだけでよい（N0 で題名を本文にそろえる）．要るようになったら後で C5 として直す．

- **最初の移行は (b)（今の 32 sample の閾値を残す）．推奨．Sol も支持．**
- (a) を後で採るなら，独立した契約の変更（C5）として，両 Provider を同時に変える．決めること（Sol）：chain 全体の長さ，packet の最小（開いた最初の send 32，閉じる 4），payload の identity（どの予約の何番目か），予約の境で footprint をどう変えるか．「境をまたげる」だけでは足りない．予約の境をまたぐ最初の send は実機で一度確かめる（INFERRED）．

### 11.8 Stop がどこで閉じるか

**決定（owner 2026-11-08，§11.1 の D′ の決定と同じ）：最初の移行は (b)．(a) は今は採らない（C4 は owner が後で承認した時だけ）．**

- **最初の移行は (b)：実際に発行した所で閉じる．** owner が D′ の決定（§11.1）で選んだ．今の振る舞いと同じ．
- (a)（規則の cursor c_L まで送る）は，finding 1〜3（実行の状態，configure の順序，Stop の意味）を D′ で解いた後に，owner が追加の送信の意味を明示して承認した時だけ，C4 として両 Provider を同時に変える．その代価（§5.4.8）：Stop の後にまだ発行していない payload を足して送り，stall があれば c_L − c_I は最大で W．窓の中の未開始の予約を始めるかを決める．stall や underflow の時に c_L を保証しない．RM-16，UR-23，MR-25，KL-13 が実質変わる．推奨：今は採らない．

### 11.9 X5・X6：継ぎと遅い継ぎを残すか

**決定（owner 2026-11-08）：両方残す．**

推奨：両方残す（Sol も支持）．Sol の注意：Vision §22 が求めるのは反応する送信で，J(e) = 5 ms という値ではない．第 1 版の「W が 5 ms 程度に下がれば X5 の代価はほぼなくなる」は確かめていないので取り下げる．Phase 10 の Reactor の要る lead が分かった時に見直す．あわせて，Stop を使う W の測り方を実機の項目に足す（handoff §2 の 2 の宿題）．

### 11.10 （新）差し替えの outcome を，境目を送ったかで決めるか（C2）

**決定（owner 2026-11-08）：決める（C2，移行の後の契約の変更）．** `applied` は境目を運ぶ操作が送れて装置の時刻が境目を過ぎた時だけ．境目の前の Stop・short は `cancelled`，境目を運ぶ send の例外は `uncertain`（新しい値）．

- **推奨：決める．** `applied` は境目の sample を運ぶ操作が送れて，装置の時刻が境目を過ぎた時だけ．Stop が境目の前で閉じた時と，境目より前で short になった時は `cancelled`．境目を運ぶ範囲が例外で終わった時は `uncertain`（新しい outcome）．利点：今の記録は二つの場面で，新しい波形が出ていないのに `applied` と書く（§5.4.5：Stop が handed cursor と B の間に来た時，境目を運ぶ send の例外）．D′ はその事実を持つので，記録が正直になる．欠点：`switches` の行の値が一つ増え（schema と RM-37・UR-38・MR-42 の文），Mock の Stop の行も同時に変える．
- 代わりの案：今の規則（Stop は B と比べる，例外は applied）を残し，「applied は記録が適用したという意味」（計画の再確認 Q-2 の裁定）と文書に書く．

## 12. 段階の素描（テスト計画はまだない）

方針（Sol finding 8）：今の振る舞いを変えない抽出を先に，契約の変更を後にする．契約の変更は両 Provider を同時に変える．各段階は tree を緑のまま残し，spec と KNOWN_LIMITATIONS の文は振る舞いを変える段階と同じ commit に入れる．

| 段階 | 中身 | 振る舞い | risky（AGENTS §9 の手順 1） |
|---|---|---|---|
| 0 | 済み：§11 はすべて決まった（owner 2026-11-08）．第 2 版への Opus の再確認（FAIL）の finding は第 3 版に入れた．次は第 3 版の §5.4 への narrow な再確認（Opus），その後 N0 の brief | — | — |
| N0 | X1（差し替えの印は SendAsapAndFlag にだけ），§11.2 の文と KL-36 の題名 | X1 は届かない枝を消すだけ | いいえ |
| O1 | spec の oracle（§5.4.11）を test support に足し，**今のコード**で通す（Mock と FakeDevice の上の UHD）．production は変えない | 変えない | いいえ（試験だけ）．ただし後の段階の安全網なので，Sol の test-plan レビューを受ける |
| E1 | 予約と Stop の置き場所を一つに：受け付けた予約（今の `Held` の中身）と Stop を，受付の順の一つの列として状態機械に置き（予約は開始時刻を key にした ordered map，§11.6），Own の `held` と mpsc の `TxCmd::Burst`・`TxCmd::Stop` をやめる（M3，§5.4.4 の 6）．`Provider::stop` も同じ列．mpsc には thread の `Shutdown` だけが残る．Own は lock の中で列を受付の順に読む．受付の now を lock の中で読む．Mock も受付で同じ列を使い，自分の `held` をやめる（§5.4.3） | 変えない | はい（ownership，queue） |
| E2a | 操作と結果の protocol：`Send` と `Close` を token で発行し，結果と報告を状態機械に一度だけ当てる．Own の記録への書き戻し 15 か所と `playing` の 2 か所が `complete` と `report` の遷移になる．発行の now は lock の中で読む．保持した尾は発行した sample で，次の `Send` か `Close{尾＋0}` が運ぶ（Stop も，§5.4.8）．BurstTracker は今と同じく最後の buffer の結果で尾まで数える（§5.4.2）．in-flight の結果は Stop の前に当てる（§5.4.4 の 4）．打ち切りは自分の op の device burst だけを終え，記録の範囲は今のまま（§5.4.4 の 2）．差し替えは受付の時に発行した cursor で取り込み，outcome は今の規則のまま（§5.4.5）．Mock は Stop・差し替え・結果の遷移を同じ状態機械に任せる | 変えない．ただし今の UHD の時刻しだいの差（取り込みの前の Stop，S12 の移動）が Mock の答えに決まり，§4 の今の不具合の見込みは構造的に消える | **はい．最も risky**（結果の ownership，記録の切り替え，装置の呼び出しの順序が一度に動く．Sol finding 8） |
| E2b | `Configure` と `Reopen` を操作に．payload の発行の前提にし，結果の後に lock の中で見直す | 変えない（今の configure の turn と同じ順序） | はい（寿命） |
| E3 | 区切りの純粋な判断（最初の send の規則，詰め物，保持の尾，send の大きさ，差し替えの境目での切り）を `ezsdr_radio` に移す．UHD の状態機械は real-time 実行役の区切りに profile の装置の値で使う．Mock の stepped 実行役の発行と block（MR-15）は装置の最初の send の最小に縛られない．Mock の `tx_handed_end`（MR-25 の第二項の模型）を，同じ区切りの関数に send の大きさの上限なしで問う形に替え，Stop の sample を下限に残す（§5.4.8） | 変えない（MR-25 の振る舞いと同じ．W = 0 でも） | はい（fake と実機の境，parity） |
| B1 | 実機：2026-11-09 の (b)〜(g) と同じ項目を D′ の上で．Stop を使う W の測り方 | — | — |
| C1 | KL-38 の撤回（§11.4 の C1：configure の拒否の後，発行していない piece を区切り直す）．予約の境をまたぐ最初の send を FakeDevice と実機で確かめてから | 変える（UHD だけが拒否を持つ．Mock は変わらない） | はい（queue） |
| C2 | 差し替えの outcome を境目を送ったかで決める（§11.10）．両 Provider と schema を同時に | 変える | はい（寿命） |
| C4・C5 | Stop (a)，短い頭への継ぎ (a)：owner が後で承認した時だけ，それぞれ独立に両 Provider を同時に | 変える | はい |
| C6 | 打ち切りの後の範囲を縮める（§5.4.4 の 2）：送った所とその閉じ方までにし，残りは装置の状態として late policy．owner が後で承認した時だけ（2026-11-09，それまでは KL-39 の Ceiling）．E2a の後なら状態機械が「送った」を持つので安い．Mock には送信の故障の模型がない（M5）ので UHD だけの規則になる | 変える | はい |

D′ の段階は送信の行（N47S5 の 185 本と N48S1・N48S3・N47S4 の送信の分）の多くを付け替える．AGENTS §9 の Maintainability の 3 に従い，状態機械の判断は純粋な遷移の表で試し，Own と Mock のテストは配線と順序（結果，報告，lock，装置の道）に残す．E2a の前に，消えるテスト・移す assert・行の対応表を試験計画に入れる（設計メモ 48 §2 の 4 と同じ）．O1 の oracle と FakeDevice の規則は E1〜E3 の間ずっと緑のまま保つ．lock の最長の待ちの記録（§5.4.9）は E2a で入れ，E3 と B1 で見る．

### 12.1 brief に入れる後始末（第 3 版の再確認の minor，2026-11-08，PASS WITH MINORS）

- **E2a**：§5.4.5 の取り込みの段落に条件「流れている repeat の device burst の中に c_I がある時」を足し，表の N2 の行とそろえる．
- **E2a（owner に諮る）**：N2 の refused の出し方．今は受付で受けて `switches` の行を `pending` で書き（`control.rs:578-590`），後で Own が refused と `COMMAND_REJECTED`「not taken」を出す．受付の時に拒むと行は書かれず（`control.rs:572-577`，今の「no repeat playing」と同じ），利用者には submit の返事の `Rejected`（O2）で届く見込み（INFERRED）．orchestrator の推奨は受付の `Rejected`（原則：受付で決まることは受付で答える）で，§5.4.7 の差の五つ目に足す．EA-16 と照らして brief で決める．
- **E2a**：§7 の RM-37 の行の「owner moves it … (UR-38)」は，消す UR-38 の移動の文と同じ commit で直す．`install` の引数 `moves` と `moved_to` の欄（両 Provider で常に null）を残すか消すかを決める．
- **E1**：`ProviderStop` に決めると `ClockEnded` の道（`end_clock` の held の取り消し，`cancel_unstarted`，`tx.rs:337-338,384-386,1161-1162`）が届かなくなる．`ur_23_a_clock_end_cancels_every_held_burst`（`tx.rs:3919-3937`），`ur_25_a_burst_booked_on_an_ended_clock_is_cancelled_when_received`（`tx.rs:3938-3955`），`CancelReason::ClockEnded` の doc（`crates/ezsdr-radio/src/lib.rs:118-119`）と mutation 行を直すか消す（AGENTS §9 Maintainability 4）．今の `ClockEnded` の `COMMAND_REJECTED` は RM-16（`design/07-radio-model.md:169`）とむしろ食い違っていた．
- **文**：§5.4.4 の 6 の `Provider::stop` の引用から UR-38 を外す（UR-23・UR-26 だけ）．

## 付録：調べ方と，確かめていないこと

- 「Easy API に `send` がない」：`grep -n "def send\|def repeat\|def switch\|def stop" python/ezsdr/*.py`（`repeat` は `session.py:470`，`switch` は :496，`stop` は :315 と :737．`send` はない），`grep -rn '"verb": "send"' python/`（何も返さない）．
- 「Mock に送信の故障の模型がない」：`grep -rn "Underflow\|underflow" crates/ezsdr-mock-radio/src/*.rs`（何も返さない）と `grep -n "TX_UNDERFLOW\|TxUnderflow\|underflow" crates/ezsdr-mock-radio/src/lib.rs`（何も返さない）．故障の種類は `lib.rs:758-776` の受信の overflow と sequence，ほかに loss．
- 記録を書く所の数：`grep -n "stop_at(\|busy\.\(advance\|install\|refuse\|cancel\|drop_piece\|move_\|close\|continuation_open\|halt\|stop\|book\|accept\|place\|prune\|switch\)" crates/ezsdr-radio-uhd/src/provider/*.rs` の本体の行（テストを除く）．
- lock：`core.rs:5`（`std::sync::Mutex`），`:252-256`（装置を呼ぶ間は持たない），`:786`（`streams: Mutex<Streams>`），`rx.rs:532-537`（受信の block ごと），`control.rs:616-644`（`check_device` は持たない），`control.rs:455,506`（`book_burst` は lock の前に now を読む），`:280-281`（値の受付は lock の中で読む），`authority.rs:143-144`（now は単調．`fetch_max` は :144．第 2 版の :142-143 は一行ずれていた）．
- テストと行の数：`crates/ezsdr-radio/tests/staging.rs` の `#[test]` 78 本，`crates/ezsdr-acceptance/tests/parity.rs` の `s*_parity_*` 19 本，`tx.rs` の `#[test]` 202 本のうち段階 5 の名前の 41 本，`mutations.json` の N47S5 185 本（staging.rs 70，tx.rs 30，control.rs 15，Mock 19，`tests/common/iii.rs` 34 ほか）．
- 外の先例は §5.11 の各 URL を 2026-11-08 に読んだ（WebFetch）．UHD 4.10 の `tx_streamer_impl.hpp`（fragment ごとの timeout）と `metadata.hpp` の `async_metadata_t`（報告の欄）も同じ日に読んだ．taprio の admin から oper への切り替えは v5.2-rc2 の source で確かめた．今の kernel で細部が変わっている可能性はある（2023 年の cycle-time-extension の修正，[lkml](https://lkml.iu.edu/hypermail/linux/kernel/2305.3/07656.html)）が，ここで使った形には関わらない（INFERRED）．
- Sol の方向レビューの bench の raw の抜き取り（`~/ezsdr-bench/2026-10-28/…/measurement.json`）は Sol が確かめた．この稿は bench-results の要約の行を引いた．
- 第 3 版で読んだ所（HEAD `134b273`，VERIFIED）：保持の尾 `tx.rs:658-662`（`hold` と `sending = count − 1`），`:663-683`（継ぎの前に尾を送る），`:764`（`len: count`），`:791`（`tracker.on_block`）．Stop の閉じ `tx.rs:823-845`（`end_open`），`:849-866`（`close_device_burst`：尾＋0 か 0 が二つ），打ち切り `:976-1020`（repeat だけ `cancel_switch_from`・`stop_at`・`playing = None`，`:994-998`．`:1008` で `close_device_burst(None, …)`）．`core.rs:399-403`（`stop_at` が `busy.stop` と `busy.halt` を呼ぶ），`staging.rs:346-349`（`halt` は `start < t` の chain だけを残す），`staging.rs:389-402`（`install`：`cursor_k <= boundary` なら境目で，:391），`staging.rs:358`（`installed && boundary < stop`）．`tx.rs:285-292`（mpsc の drain，`Shutdown`），`:326-343`（`TxCmd::Burst` は差し替えでなければ記録を見ずに `held` に入る，`TxCmd::Stop` は `stop_now`），`:451`（step の now は lock の外），`:603-606`（configure の拒否は `COMMAND_REJECTED`），`:933-936,948-951`（reopen の失敗は `ClockEnded`）．`control.rs:310-320`（Stop の受付は lock の中で now を読み，`stop_at` の後で `TxCmd::Stop` を送る）．`crates/ezsdr-radio/src/lib.rs:123-126`（`cancellation_reports` は `ClockEnded` だけ true）．Mock：`crates/ezsdr-mock-radio/src/lib.rs:208,216`（`held`，`busy`），`:972-985`（`open_held` は値を burst の最初の sample で使う），`:1710`（受付で `install(id, now_k, false)`）．spec：`design/09-mock-radio.md:171`（MR-25 の三つの項），`:134`（MR-15），`:221`（MR-42），`design/07-radio-model.md:168`（RM-16「a later burst is transmitted as usual」），`:346`（RM-37），`design/18-uhd-radio.md:297-299`（UR-38 の取り込みの文）．
- 確かめていないこと（第 3 版で足したもの）：再確認の M3 の列と §4 の今の不具合の見込みは，コードを読んだ推論で走らせていない（INFERRED．頻度も分からない）．打ち切りで尾を捨てた時に記録が 1 sample 多く数えること（INFERRED，§5.4.2）．受付の時に取り込めば S12 の移動が Provider から届かないこと（INFERRED，§5.4.5）．開始時刻の ordered map で重なりの判定が足りること（INFERRED，§11.6 の owner の注）．Mock の送信の道（`lib.rs:1159-1210`，`1577-1830`）は MR-25・MR-42 に要る行だけを読んだ．
- 確かめていないこと：D′ の lock の保持と待ちの上限（実装がない，INFERRED）．予約の境をまたぐ最初の send を装置が同じに扱うこと（INFERRED，C1 と §11.7 (a)）．FakeDevice の `device.rs:689` 以外の 4〜32 の front の根拠（pending (c)）．20 000 sample の send の最悪の止まり方（fragment の数からの算術，INFERRED）．oracle の大きさ（数百行の見込み，INFERRED）．FakeDevice の `transmitted` を多 channel に広げる手間．srsRAN の 0 埋めの上限の値（ヘッダの説明だけを読んだ）．D′ の行数の見込み（作っていないので数えていない）．

## Changes

| 日付 | 変更 |
|---|---|
| 2026-11-08 | 下書き（Opus subagent，tree の外）．owner の追加の依頼（A〜C の外の案，契約を狭める案，外の先例）を §5.5〜§5.11 に入れた |
| 2026-11-08 | 第 2 版：§11.1 は D′ に決定．Sol の方向レビューの finding 1〜9 を §5.4（D′ の設計の文）と §12（抽出を先に，契約の変更を後に．最も risky は E2a）に入れ，§11 を Sol の立場で改めた（X4 は退ける，KL-38 は X2 でなく C1，§11.7・§11.8 の最初の移行は (b)，§11.10 を足した）．事実の訂正：D が頼る事実は VERIFIED だけではない（§8），`SEND_TIMEOUT` は fragment ごと（§5.8），streams の lock は装置を呼ぶ間持たない（§5.2 (b8)，§5.4.9），封じの式は strict（§5.4.10），Stop (a) の追加の送信は send 一つ分とは限らない（§5.4.8） |
| 2026-11-08 | owner が §11.2〜§11.10 を決めた（orchestrator が各節に決定を書いた）：§11.2 (a)，§11.3 X1 を採る，§11.4 C1，§11.5 X3 は採らない（11-04 の決定を残す．第 2 版の推奨を改めた），§11.6 X4 は採らない，§11.7 (b)，§11.9 両方残す，§11.10 C2．§11.8 (a) は D′ の決定どおり今は採らない．§1・§2・§7・§12 の X3 の記述を直した |
| 2026-11-08 | 第 3 版：第 2 版への Opus の再確認（FAIL，方向と決定はそのまま，`~/.cache/ezsdr-fixes/2026-11-08-n49/recheck-rev2.md`）の finding をすべて設計の文に入れた．M1：保持した尾は発行した sample で，Stop の `Close` が尾＋0 を運ぶ．Kernel の burst の記録は今と同じく最後の buffer で尾まで数える（§5.4.2，§5.4.8）．M2：E2a では差し替えを受付の時に発行した cursor で取り込み，outcome は今の規則．MR-42 の文の変更は C2 へ．UHD の時刻しだいの差は Mock の答えに決まる（§5.4.5，§5.4.7，§7）．M3：E1 で Stop も予約と同じ受付の順の列に入れる（§5.4.4 の 6，§12）．M4：最初の send の 32 は装置の規則で real-time 実行役だけ．Mock の Stop の切りは MR-25 の式で Stop の sample を下限に残す（§5.4.3，§5.4.8，§5.4.10）．M5：打ち切りの後の記録の範囲は今のまま，発行していない継ぎの piece は発行の時に late policy（§5.4.4 の 2）．m1：in-flight の結果を当ててから Stop が取り消す（§5.4.4 の 4，§5.4.6）．m2：`next` の now を lock の中で読む（§5.4.3）．m3：Mock は E1 で列，E2a で遷移，E3 で区切り（§5.4.3，§12）．m4：`apply_due` の待ちは stepped 実行役に当てない（§5.4.3）．決定とのずれの残り（Status，§2・§7・§10 の見出しと列，§10 の X3 の行，§11 の冒頭，§11.8 の決定の行，§12 の段階 0）を直した．`authority.rs` の行を :143-144 に直した．§4 に今のコードの不具合の見込み（INFERRED，issue は orchestrator が立てる）を足した |
| 2026-11-08 | 第 3 版の再確認（FAIL：N1，N2 と minor 3）を orchestrator が直した：Stop の受付の答えはすぐ効き，結果を待つのは報告と発行の列からの除去だけ（§5.4.4 の 4，N1）．流れている repeat がまだ発行されていない時の差し替えは今と同じく refused（§5.4.5 の表，N2）．`install` の移動の枝と UR-38 の移動の文は E2a で消す．§5.4.7 の決定的になる差を四つに．`Provider::stop` の取り消しの理由を E1 で `ProviderStop` に（§5.4.4 の 6） |
| 2026-11-08 | 再確認（Opus，narrow）：PASS WITH MINORS．N1・N2・2.a〜2.c は閉じた．minor 5 件を §12.1 に（E1・E2a の brief で拾う．N2 の refused の出し方は owner に諮る）．設計の段はここで終わり |
| 2026-11-09 | orchestrator：2026-11-09 の実機の結果を §8 に入れ（前線の +1 と Stop の c＋3・c＋4・J(e) の継ぎ・時刻なし・Stop の終わり・送信の rate・ssh 切断は VERIFIED．Stop の後の詰めた頭の c＋3 は記録の誤り〈#82〉，Module の前線の burst の飢え〈#81〉），tree に入れた（`plan/phase8/49-time-and-booking-inventory.md`） |
| 2026-11-09 | orchestrator：段階 B（#81・#82）の後．§3 の R3・R6・R8・U2 の前線は「32 sample 以下の閉じた device burst の後は詰めた終わり＋10」に変わった（打ち切りと T3 flush の閉じも＋10，Stop が残す・落とす短い頭は自分の前線まで．記録 `reviews/stepB-dispositions.md`）．D′ の区切りの関数はこの値を profile の値として受け取る（§6 の 6） |
| 2026-11-09 | orchestrator：N0 の Sol のコードレビュー（FAIL）で，RM-15 の文の二つの項が今のコードと違うと分かった：遅れの判定が重なりより先（遅れた DropAndFlag・RejectAtPlan は重なりを見ずに落ちる），Stop や打ち切りが切った chain は境まで記録に残り Stop の範囲を作らない（`staging.rs:385-391`）．文はコードに合わせた．打ち切りの後の残りの範囲を予約の footprint として拒む今の振る舞いは，§11.2 の Sol の注意（打ち切りの残りは late policy）とずれる．owner の決定（2026-11-09）：今は KL-39 の Ceiling として残し，範囲を縮めるのは §12 の C6（後で承認した時だけ） |
| 2026-11-09 | orchestrator：N0 は済（X1，RM-15 の footprint の文，KL-39．記録 `reviews/n0-dispositions.md`，gate PASS）．re-check の note から #84（dispatch で落ちた repeat の保留の差し替えが cancelled でなく refused．E2a で直す．O1 の oracle は spec の答えで書き，既知の食い違いの印を E2a で外す） |
| 2026-11-10 | orchestrator：O1 は済（記録 `reviews/o1-dispositions.md`）．oracle は spec の文から書き，今のコードの三つの食い違いを issue にして固定した：#84（N0 で立てた），#85（Stop が dispatch 済みで未送信の burst を記録なしに消す．E1・E2a で一つの列から取り消す），#86（動かした差し替えの記録の `requested_target`．どの値かは E2a で RM-37・UR-38 の文として決める）．RM-16・MR-25 の Stop の範囲と MR-3・RM-15 の Mock の J(e) の allowance は文をコードに合わせた．次は E1．E2a の前に実機（handoff §2） |
| 2026-11-10 | orchestrator：E1 は済（記録 `reviews/e1-dispositions.md`，形の素描 `reviews/e1-sketch.md`）．§12.1 の E1 の見込みのうち二つは違った：`cancel_unstarted`（ClockEnded）は reopen の失敗の後の受付で今も届く（残した．消したのは `end_clock` の取り消しと clock の不一致の枝），`continuation_open` の継ぎの節は step の未読の確認と close の間に継ぎが受け付けられうるので残る（Sol）．step の確認は未読の halt だけ（予約の未読でも読むと差し替えの取り込みが早まり，UR-38 の移動の道が E2a より前に消える）．#85 は E1 では直らない（送る前の burst は列の外の `open`），E2a で直す |
