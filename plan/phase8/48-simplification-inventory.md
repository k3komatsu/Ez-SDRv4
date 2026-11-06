# 設計メモ 48：理由がなくなった仕組みの棚卸し（note 47 の後）

Status: 第 2 版（2026-11-05，orchestrator Opus）．第 1 版に Sol のレビュー（[reviews/48-sol.md](reviews/48-sol.md)，NOTE NEEDS CHANGES）をすべて入れた．§5 の三つは owner が推奨どおりに決めた（2026-11-05）．次は §4 の S1．

## 1. 目的と範囲

note 46 と 47 で，次のことを決めた．
- Run の中の radio の値の変更はやめた（段階 2）．
- 受信の値は，止めた stream にだけ受け付ける（段階 3）．
- 送信の値は，空の stream に受け付ける（段階 4，未着手）．
- 送信の一般の割り込みはやめ，repeat は次の周期の始めで差し替える（段階 5，未着手）．

それより前に，UHD Module・共通の timeline・MockRadio・FakeDevice・spec 07/09/18 に入れた仕組みがある．その中から**入れた理由がもうない物**を洗い出し，消す・残す・まとめるを決める（owner 2026-11-05）．

**きっかけ**：2026-11-05 の実機で，Module を通る送信が underflow と TIME_ERROR を出した（#78）．#72（流れている burst の中の時刻つきの送信の周波数）のために入れた「送信のすべての send に時刻」（D9b-1a）は，#72 の前提が note 47 で消えたので理由がない．ただし実機の log を読み直すと，#78 は少なくとも二つの別の不具合だった（#78 のコメント）：(1) 開いた時の T3 flush が引き起こす Underflow の報告が Run の `TX_UNDERFLOW` に漏れる，(2) 前の burst の終わり＋1 サンプルの burst を Module が 1 サンプル遅れとして捨てる（D9b-1a の 2 サンプルの close の後の安全な開始の判定，INFERRED）．どちらも「すべての send に時刻」では説明できない．

**範囲の外**：
- 一律の書き直し（owner：理由が消えた物だけを対象にする）．
- 段階 4・5 が自分で消す物（§3.4）．
- Module のスレッドの形（設計メモ ④）．

**事実の出典**：[research/48-inventory-facts.md](research/48-inventory-facts.md)（Sonnet の読み取り，HEAD `9dab08a`，path:line と grep，VERIFIED/UNSURE つき）と Sol のレビュー．表の番号は事実の表の番号を使う．この文書は判断を書く．

## 2. 判断の基準

一つの仕組みは，次の順に決める．
1. **本番の操作から届くか**．本番の操作は，connect の値，staged の値つきの `start_rx`，Stop，send・burst，repeat，loss・fault，`Provider::stop` を指す．届かない物（試験・実機の probe・測定の道具からだけ届く物）は**消す**．試験の道具として要るなら，本番の型の外（試験の側）へ移す．
2. **届くが，入れた理由がもうないか**（その理由を作る操作が消えたか）．それなら**消すか簡単にする**．残る用途があれば，その用途に合う最小の形にする．
3. **理由が残るか**．それなら**残す**．名前や説明が古いだけなら直す．
4. **消す物の試験**：試験は「何を確かめるか」で分け，場所でまとめて消さない．消す仕組みの試験の中にある，残る振る舞いの assertion は残すか移す．実装の前に，各段階の試験計画で「残す試験・移す assertion・消す試験」の対応表を作る（Sol）．

## 3. 決定の表

### 3.1 消す（本番から届かない）

| # | 仕組み | 理由が消えた根拠 | 消すもの | 残すもの・注意 |
|---|---|---|---|---|
| A1 | `Device::apply` の `at`（時刻つきの設定）．`UhdDevice::apply` の `set_command_time` の囲み．FakeDevice の時刻つきの命令の queue（`commands`，`settle`，`FakeConfig::command_queue`，`apply` の back-pressure）．DSP の FIFO（`DSP_FIFO`，`Dsp.fifo`，`drain`）と，それに仕える flow の部品（`open_flow`，`close_flow`，`end_rx_flow`，flow の cursor，時刻つきの履歴の印．Sol 欠 3：`sample_at` は使わない） | 本番の `apply` は `core.rs:1031` の `None` だけ（VERIFIED，Sol も確認）．時刻つきの値の変更は段階 2 で消えた | trait の引数，UHD と Fake の時刻つきの枝，UR-2・UR-33 の時刻つきの部分，試験を転送する Device（probe 等）の時刻つきの枝 | **残す**：時刻なしの設定，`apply_delay`，coercion・read-back・拒否の試験，周波数の履歴（`device.rs:1464-1519,1148-1155`），送信の古い header の状態（`device.rs:1980-1992`）と P5/D70 の packet の状態の試験．実機の D9 probe（`tune_dsp`・`tune_auto`）は `UhdDevice` の bench 用の関数として残し，trait には置かない |
| A1′ | `Device::rx_stop` の `at`（Sol 欠 1） | 本番は `None` だけ（`rx.rs:287,405`）．X3x0 は時刻つきの STOP を無視する | trait の引数．時刻つきの STOP の raw probe は trait の外 | UR-2・UR-33 を直す |
| A2 | `radio.timing.command_queue_depth` と `radio.COMMAND_QUEUE_FULL`（payload と schema） | 実行時に読む物も出す物もない（VERIFIED）．測った「7」は，高い層の時刻つきの `Device::apply` の数で，FIFO の数ではない（`tests/common/measurement.rs:1477-1479`）．残る時刻は，受信の開始（別の受信の命令の FIFO を通る．STOP はそれを素通りして消す：[UHD host](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/radio_control_impl.cpp#L1116-L1133)，[FPGA](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v#L157-L199)）と，送信のデータと一緒に運ばれる時刻だけ．だから，この key は**もう使わない能力**を表す | key，型，profile の値（x3xx 16，ideal），event kind と payload の登録，schema，RM-6/10/11/20/35，MR-3，UR-9/34 の行，測定の群 `measure_command_queue_depth` と fixture，実機の queue の probe | 開始・停止の順序と back-pressure の振る舞いの試験は残す．受信の命令の FIFO と送信のデータの道を spec に一行書く．schema を再生成して `schema_freeze` を緑にする．凍結前なので版は上げない（AGENTS §6） |
| A3 | `Line::halted`・`State.halted` | 非試験の呼び手がない（VERIFIED） | 関数，欄，それを呼ぶ試験の行 | halt は規則として残る．plan・`streaming()`・拒否の除外・prune の同値の assertion は残す（`timeline.rs:215-221,263-266,488-512`） |
| A4 | `Core.tx_dsp_gates`・`lock_tx_dsp` | 争う相手（時刻つきの DSP の書き込み）が本番にない（VERIFIED） | gate と取得 | report の gate，generation の分離，flush の ACK と期限，無関係の report の保存（`core.rs:1072-1181`）は残す |
| A5 | `Channels::without`，`Device::rx_start_now` | 本番の呼び手がない（VERIFIED） | 関数 | — |
| A5′ | `Recovery::Halted` | 届く失敗（reopen の失敗）で書かれるが，読むのは試験だけ．失敗は `clock` も消す（`tx.rs:180-183,805-855`） | 型の枝を畳む | **試験は残す**：reopen の失敗の後，控えた payload も新しい payload も送らない |
| A6 | 古い説明（空の UR-24/UR-25 の見出し，`FromNone`，RM-34 の引用，「pending switch」，Mock の `advance` の説明，`RxApplyFails` の説明，`release_window_ns` の説明） | 指す物がない | 文だけ | `switch_cut` は届く終わりの cut を指すので，**名前を直して説明し**，検査は残す（`tx.rs:512-525`）．Changes の表と撤回した規則の stub は歴史として残す |
| A7 | Mock の常に空の `applied` の節（`M/lib.rs:389`，Sol 欠 2．47 §4 の「Mock の `applied` の節（常に空）」と同じ） | 書く物がない | 節，読み手，spec，記録の試験 | staged の値と装置の read-back を混同しない |

### 3.2 簡単にする（届くが理由がない）

| # | 仕組み | 根拠 | 新しい形 | 残すもの・試験 |
|---|---|---|---|---|
| B1 | 送信のすべての send に時刻（UR-22 の T1，`tx.rs:563,594,758`） | 入れた理由（#72）は note 47 で消えた．しかし S1 の試験計画で，時刻なしの続きには別の危険が分かった：最初の send が遅れて捨てられると，時刻なしの続きはずれた時間軸で黙って鳴る（FakeDevice の `Duc::packet` でも同じ形，INFERRED）．全部の send に時刻なら，遅れた send は TIME_ERROR で捨てられ報告される．2026-11-05 の C++ の実測では，続きの時刻は #78 のどの症状とも関係しない．コードの差は `at` の 3 か所だけ | **残す**（owner 2026-11-05．理由を「遅れた後にずれて鳴るのを防ぐ」に改める．UR-22 の T1 の説明を直す） | #72 は前提がなくなったので閉じる |
| B2 | T3 flush（開いた時の時刻なし 2 サンプル） | 開く時・開き直す時の古い時刻の状態（前の一回の時刻つきの burst を含む）を消す．時刻なしの続きにしても，前の burst がすべて時刻なしになるわけではない（VERIFIED） | **残す** | P5 A/C，flush の ACK・期限・generation の試験．時刻つきの DSP の命令を流す処理と混ぜない |
| B3 | 受信の再開の ready の `release_window`（`timeline.rs:533-540`）と `TimingEnvelope.release_window_ns` | 待つはずの時刻つきの命令がない．note 47 §2.1 の 4 もこの項を消すと書いている | ready の項から**release window だけ**を外し，key・型・profile の値・RM-6 の検査・schema から消す．RM-20/25/35，MR-3，UR-3/9/25/34，測定の比較と fixture も直す | 古い rate の受信の呼び出しの時間，delivery allowance，start lead，格子への丸め，設定の完了の警告，missed start は残す．`rm_25_a_receive_successor_adds_release_window_before_call_and_lead` は，残る項を正確に確かめる試験に向け直す．MockRadio と UHD の再開の origin の一致（differential，VH）．**profile の start lead は同時に下げない**．2026-11-05 の「候補 3 ms でも遅れなし」は，消した後の 3 ms の余裕の証明ではない（残りのリード 6.7〜9.4 ms は release window の floor による）．消した後に実機で候補を測り直す |
| B4 | 送信の時間軸の計画の複数の区間（UHD：`Tx.plan: Vec<Planned>`，`version`，`retarget`，`end_clock`，`cut`．Mock：`TxStream.clocks: Vec<TxClock>`，`cur`，`tx_gen`，`tx_reconcile`） | 送信の line は Stop と End しか持たず，計画は高々一区間（VERIFIED） | **note 47 の段階 4 で決める**（§5 の 1）．現在の clock は一つ．過去の clock の記録と報告の id は使えるまま | origin より前の loss・stop で booking の時に registry の clock を終える（`core.rs:669-679`），取り消し，Manifest の履歴．「失敗 → 直す → 送り直す」の系列 |
| B5 | `CancelReason::ColdChange` | 作られるのは，clock が origin で終わった時と reopen の失敗だけ（`tx.rs:348,828,844`）．`cold` の変更は何も取り消さない | 名前を `ClockEnded` などにし，説明を直す | `cancellation_reports`，event の数，取り消しの記録は変えない |
| B6 | Mock の送信の値の履歴（`ChannelModel` の三つの Timeline，`M/channel.rs:56-62`．本番の `.set()` は受信だけ．Sol 欠 4） | 今は値が変わらない | 段階 4 の形と一緒に決める | 受信の履歴は残す |
| B7 | `owner_step` の「channel の組が変わった時の開き直し」と「同じ命令なら何もしない（`Nothing`）」の枝（Sol 欠 5，C4） | 受信の streamer は arm で開き，組は変わらず，受け付けた再開は毎回新しい番号を持つ（`mod.rs:555-560`，`core.rs:614-625`，`rx.rs:362`）．今の本番から届くことは示されていない（INFERRED） | 届くかを実装の段階で辿り，届かない枝は消す | 設定・拒否・loss の順序は残す．所有の仕組み全体は書き直さない |

### 3.3 残す（理由が残る）

| # | 仕組み | 残る理由 |
|---|---|---|
| C1 | `RxChain`（`Off`，`Armed`，`Streaming`，`Uncertain`，`Lost`） | 開始，再開，最初の sample，停止の失敗，loss のどれも本番から届く（VERIFIED）．`abandon_unowned` は，`rx_start` が成功した後に owner のスレッドの生成が失敗した場合（`mod.rs:635-650`）．片付けと「loss の後は呼ばない」の試験を残す |
| C2 | `TxChain` の device burst と front，`Tail`，閉じる send の 2 サンプル，reopen と halt（F8） | 隣り合う burst，Stop，有限の終わり，失敗した close で使う（`tx.rs:444-480,543-575,715-855`）．#70 と F8 の決定（46 §8） |
| C3 | 報告の照合（`PendingReport`，generation），`tx_in_flight_ns` | 報告の照合（UR-28）．in-flight の窓は Stop の payload を抑え，段階 5 の repeat の差し替えの境も守る．割り込みと一緒に消さない．Mock の Stop の模型も残す |
| C4 | `ColdConfig`（名前は `RxConfig` などに直す），`prune`，`refuse`，`made`，`Staging`，`lead_left` と `START_LEAD_SHORT`，`min_timed_command_lead_ns`，`Owner`・`Claims` | 段階 3 の staging と再開，予約の時刻の規則．届くかの示されていない分岐は B7 |

### 3.4 段階 5 に任せる

- **送信の一般の割り込み**（`Tx::preempt`，open burst の held の切り）：今日も届く（`tx.rs:431-457,957-1012`）．段階 5 が重なる予約を拒む時に消す．
- **残すもの**：`dispatch_start`，`later_bound`，`late_policy_front_outcome`．重ならない正しい予約でも，lead が切れるまで待つ，あるいは前の close の安全な front より前に始まる，ということがありうる（`tx.rs:479-485,904-945`）．消すと，遅れた予約や間隔の近い予約が誤った時刻に送られる．`ur_21_first_dispatch_rechecks_all_late_policies` と，EOB の後の安全な開始の試験を残す．
- **Mock**：割り込みのための front と，Stop・安全な front のための front を分け，後者を残す（`M/lib.rs:1342-1376`）．
- **送信の line の `Kind::Stop`**：計画では何もしないのに `Streams.version` を上げる．これは B4 と一緒に見る（owner への知らせと記録は残す）．

### 3.5 足す（装置の性質を Mock に）

受信の再開の先頭を FakeDevice と MockRadio（profile の値）で再現する（owner 2026-11-05）．2026-11-05 の実機では，1 MS/s で約 13 の厳密な 0 と約 10 の立ち上がりが出た（DDC のフィルタ，INFERRED）．rate（間引きの比）ごとの数を次の実機で測ってから決める．消す段階とは分ける．

## 4. 段階

1. **S1：#78 を直し，送信の時刻を device burst の最初の send だけに戻す（B1）**．
   - risky（fake と実機の境，送信の順序と報告）．試験計画を書き，Sol のレビューを受ける．
   - 順番：(a) T3 flush の Underflow の報告を flush に属させる（flush の send からその ACK までの報告は Run の `TX_UNDERFLOW` にしない）．(b) 前の burst の終わり＋1 サンプルの burst を Module が遅れとして捨てる判定を直す．(c) `hw_p70_…` の TIME_ERROR と `L` を調べる（説明がつくまで直すとは言わない）．(d) B1．どれも先に FakeDevice で再現する．
   - B1 の名指しの試験を入れる．次の実機で #78 を確かめるまで #78 は閉じない．#72 は B1 で閉じる．
   - **済**（2026-11-05，記録 [reviews/48s1-review.md](reviews/48s1-review.md) と `reviews/48s1-*`）．device burst の最初の send は channel ごとに 4 サンプル以上（1〜3 サンプルの burst は 4 まで 0 で詰め，front は開始＋5），T3 flush は 4 つの 0 で，その channel の報告は ACK か期限まで flush のもの，FakeDevice は短い最初の packet と終わり＋1 tick の ACK を模す，ACK の照合は `<`，Mock は詰めた front を Stop の後も持つ，window の空きを待つ最初の send は lead に入ったら late policy で決め直す．B1 は owner が外した（すべての send に時刻を付けたまま）．受付で決める Mock と送り出しで決める Module の食い違い（短い burst の直後 4 サンプル以内の予約）は #79 として段階 5 で直す（owner）．
2. **S2：時刻つきの設定の道を消す（A1–A7，B3，B5，B7）**．
   - trait と FakeDevice の境を変えるので risky．
   - 試験計画に，残す試験・移す assertion・消す試験と mutation 行の対応表を入れる．
   - 試験の削除が大きい（`device.rs` の試験，differential の時刻つきの部分，測定の群）．
   - 試験計画（`reviews/48s2-test-plan.md`，Sol と Opus の再確認を経た第 3 版）で S2a-i-α（queue の測定の群）・S2a-i-β（`apply`・`rx_stop` の `at`，FakeDevice の queue と DSP の FIFO）・S2a-ii（A4，A5′）・S2b-i（A2 の残り，B3）・S2b-ii（A3，A6，A7，B5 は `ClockEnded`，B7）に分けた．**S2 は済**（2026-11-05〜06，記録 `reviews/48s2-*`，数え方は `48s2-dispositions.md` の末尾）．受信の再開の lead は次の実機で測り直す．`StreamState.configured` は本番の判断で読まれなくなった（残すかは後の段階で）．
3. **次の実機**（handoff (7) ③′）．
   - 200 MS/s の Module の負荷，延ばした (c)・(d)・(e)・(f)，#1 の送信の rate，#78 の確認．
   - 再開の先頭の rate ごとの数，B3 の後の受信の start lead の候補．
   - **済**（2026-11-06，準備の記録 `reviews/bench-iii-*`，結果は [bench-results](bench-results.md) §「2026-11-06」）．#78 の残りの原因が分かった（下の S3）．残り：`hw_p70` の「送った後に予約」の形（予約の時に B が装置の lead の中にある，試験の欠陥と読む），#2 の送信の rate の onset（±11 µs，閉じ方は owner），200 MS/s の送信が安定しない（6 回中 3 回），#1 の受信の feed の drop（46 block），(g) ssh 切断は sshd を起こしてから．
3′. **S3：#78 の残り**（**済**，2026-11-06．記録は [reviews/48s3-dispositions.md](reviews/48s3-dispositions.md)．D1（owner）：32 サンプル以下の burst のちょうど次の予約は続けず late で判定（F7 (b) を狭める，KL-36）．Mock は note 40 D5 のまま（end＋1 は Mock が 1 サンプル厳しい）．前もって届く予約のずれ（後の予約で 4〜32 サンプル目から切る形を含む）は #79 で段階 5 へ．実機で確認済み）．
   - 装置の規則（VERIFIED，2026-11-06）：補間 80 以上で，開いたままの device burst の最初の send が 28 サンプル以下だと，続きが後から来れば Underflow．30 以上は起きない．補間 50 以下は 2 サンプルでも起きない．S1 の「4 以上」は閉じる send がすぐ続く形だけの規則だった．
   - 直し：開いたままにする最初の send を 32 サンプル以上にする（窓に 32 入るまで渡さない）．32 以下の burst は一回の SOB＋EOB で送り，終わりにちょうど続く予約は続きにせず late の規則で判定する．1〜3 の 4 への詰め物は残す．repeat の最初の send は 32 まで並べる．
   - FakeDevice に装置の規則を入れ，直す前に再現する．MockRadio の MR-17・MR-25 を合わせる．#79 の範囲が 4 から 32 に広がる点は試験計画で確かめる．
   - 直した後の実機（2026-11-06，`de7720c`）：(b) は `hw_p70` の「送った後に予約」（試験の形の欠陥）を除き二台とも PASS，`hw_s1` は二台とも 9 回 PASS（[bench-results](bench-results.md) §2026-11-06（続き））．
4. **note 47 の段階 4**（B4 と B6 を含む．**段階 4 は済**，2026-11-07，記録 `reviews/47s4-*`：B4 は一区間に，B6 は値の履歴を残して burst の開始の時刻から使う）**と段階 5**（§3.4．#79：短い burst は詰めた範囲 `[s, s+5)` を占めるとして，重なる予約を Mock と Module が同じ規則で受付時に拒む）．段階 6 の片付けはこの棚卸しに吸収した．

各段階は今までどおり，brief → 試験計画と Sol（risky なので）→ 実装 → Sol の code レビュー → gate の順に進める．

## 5. 決めること（owner）

1. **B4（送信の時間軸を一区間にする）をいつやるか**．**owner 決定（2026-11-05）：note 47 の段階 4 に入れる**．推奨：**note 47 の段階 4 に入れる**（Sol も同じ）．段階 4 は，設定を伴う burst の booking で新しい送信の clock を登録する（note 47 §2.1 の 3）．そこで「現在の clock は一つ．過去の clock の記録は残す」を一度に設計する方が，先に畳んで段階 4 でまた書き直すより少ない．退けた案：S3 として先に畳む．
2. **A2（queue の深さの key と `COMMAND_QUEUE_FULL` を消す）**．**owner 決定（2026-11-05）：消す**．推奨：**消す**（Sol も同じ）．もう使わない能力（時刻つきの設定の queue）を表すだけで，読む物も出す物もない．受信の命令の FIFO と送信のデータの道を spec に一行書く．退けた案：記録の値として残す（読み手がない）．
3. **B3（受信の再開の release window を消す）**．**owner 決定（2026-11-05）：消す**．推奨：**消す**（Sol も同じ．note 47 §2.1 の 4 と同じ）．profile の start lead は同時に下げず，消した後に実機で測り直す．退けた案：残す（待つ対象がない）．

## 6. 確かめていないこと

- #78 の三つ目（`hw_p70_…` の TIME_ERROR と `L`）の原因．(1)(2) を直せば #78 の残りが消えるか（実機で確かめる）．
- B7：owner の分岐が今届かないこと（INFERRED．辿る）．
- 再開の先頭の rate ごとの数．
