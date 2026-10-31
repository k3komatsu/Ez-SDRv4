# 設計メモ 45：UHD Module の判断を一つの純粋な状態機械に集め，操作の順番を明示して探索できるようにする

Status: 第 5 版（2026-10-31，orchestrator Opus）．Sol のレビュー 4 回（REVISE major 7 → REVISE major 5 → REVISE major 3 → ADOPT WITH CHANGES major 1・minor 2．変更は入れた．§11）．owner は方向と使い捨ての実験を承認（2026-10-31，§8・§10）．実験は他の作業と並行せず，handoff §2 の (5)状態．移行の承認ではない．**owner 2026-10-31：実験を進める（並行にせず順番に）**．owner（2026-10-31）：根本の問題（並行処理，テストの増え方，spec の膨らみ）に正面から答える設計メモを今から書く．手間が増えても長期的に良い形を選ぶ．書き直すかどうかは証拠（§7 の実験）を見てから決め，決めたら 8b の前に移す．

| 項目 | 内容 |
|---|---|
| きっかけ | owner の依頼（2026-10-31）で Opus が出した改善案 14 件と Sol の意見（`~/.cache/ezsdr-fixes/2026-10-31-sustain/`）．三つの根本の問題が同じ所から出ているという仮説（§1） |
| 材料 | Sol の棚卸し [research/45-concurrency-inventory.md](research/45-concurrency-inventory.md)（スレッド，共有状態，lock，UHD 4.10 のスレッド安全性，実時間の予算，テスト用の hook，競合の履歴，テスト用の時計の限界），設計メモ [35](35-uhd-stream-state.md)・[37](37-test-thread-timing.md)・[39](39-dsp-timed-commands.md)・[44](44-device-chain-state.md)，[failure-mechanisms](../maintenance/failure-mechanisms.md)，main `d52f127` |
| 機構 | 並行処理から出る機構が集まる所：`stream-lifecycle-scattered`，`device-state-outlives-call`，`planned-state-read-as-actual`，`correlation-by-partial-key`，`wall-clock-test-oracle` |
| 範囲 | UHD Module（`crates/ezsdr-radio-uhd`）の中だけ．Kernel の契約，Stream Contract，MockRadio，Python は変えない |
| risky | はい：Module の中の寿命，順序，ownership，FFI の呼び方，fake と実機の境のすべてに触れる（AGENTS.md §9） |

## 1. なぜ，何を

Phase 8 の UHD Module には三つの問題がある．

1. **競合の不具合が，直しても次の所に出る**．棚卸し §7 の 13 群の多く（spec 22 の refusal の往復，1b-4c，1b-4d-1a，1b-4e-2，38a，D9b-1a，D9b-2，L1）は，二つ以上のスレッドが同じ事実を別々の時点で読み書きして起きた．ほかは teardown（Phase 7 Q），装置の模型，テストのスレッドの timing（#10・#68・#69）である．直し方は lock の範囲を広げる，flag を足す，状態の型にまとめる（設計メモ 35・44）のどれかで，L1 の受信の窓は「narrowed, not closed」のまま残った（`plan/maintenance/failure-mechanisms.md:62`）．D9b-2 は Sol の判定が 7 回，L1 は code FAIL（major 3）だった．
2. **テストが本体よりずっと速く増える**．provider の本体 6,240 行に対しテストは 20,402 行（`#[test]` 488 本），そのほか `tests/` に約 11,800 行（棚卸し §10）．競合は一つずつ，gate と probe を使う手書きのテストで固めている．本体に test 用の probe が 23 か所ある（棚卸し §6）．テスト用の時計はスレッドの走る順番を決められない（棚卸し §8：OS が決める）ので，テストは思いついた順序しか試せない．
3. **spec が実装の写しになって膨らむ**．spec 18 は 16 日で 155 KB から 326 KB．UR-25 は claim，flag，lock を規定し，内部のスレッド名が頻出する．スレッドどうしのやり取りの細部が正しさを決めるので，レビューはそれを規範に書いて固めてきた．

**仮説**：三つの多くは同じ所から出る．判断が四種類のスレッド（uhd-control，`uhd-rx-s{k}`，`uhd-tx-s{k}`，uhd-clock）に散り，lock を持つ `Core` の 9 つの field と 2 つの atomic を通して互いの事実を読む（棚卸し §1・§2）．どの判断も「lock X の下で事実 Y を読み直して決める」形で，正しさは呼び出しの場所ごとの lock の範囲と読み直しに懸かる．しかも lock の多くは装置の呼び出しをまたいで持たれる（棚卸し §2）．設計メモ 35・44 は値に型を与えたが，判断の場所は散ったまま並行に走る．「どの道も chain の lock の下で loss を読み直す」のような約束は，grep とレビューでしか確かめられない．これは仮説で，§7 の実験で確かめる．

**この設計メモで決めること**：

1. 判断を，一つのスレッドで順番に走る**一つの純粋な状態機械**（以下「判断役」）に集める．判断役は装置を呼ばず，時計を読まない．
2. 装置の呼び出しは**実行役**が行う：設定を呼ぶ実行役が一つ，stream ごとにその streamer のすべてを呼ぶ実行役．実行役は判断役から渡された権限の範囲でだけ，自分で決める（§4.3）．
3. **同期はなくならない**（Sol 2 回目の結論）．装置そのものが並行に動き（UHD の command queue で待つ，途中まで効いて失敗する，native の lock を共有する），それはこの形でも残る．この形で得るのは，**装置への操作の順番が，判断役の状態にある命令の状態（§4.4）として明示され，探索できる**ことである．判断役と実行役が出会う所は，stream ごとの小さな**受け入れの cell**，instance に一つの**失敗の印**と lifecycle の受け入れ，今の native の lock に限る（§4.6 の表）．cell は短い区間しか持たず，装置の呼び出しをまたがない．
4. 判断役と cell の手順と native の lock を，**決定的シミュレーション**（以下 DST）で，受け入れ・装置への効果・完了・判断役への到着を別々の出来事として機械的に探す（§5）．
5. 書き直す前に，一番競合の多い一切れ（受信の chain と uhd-control の受け渡し）で**実験**し，効くかを証拠で確かめる（§7）．

## 2. 今の形（棚卸しの要点）

- **スレッド**：`uhd-rx-s{k}`（受信，segment の開始，cut，DDC の値の始末，restore），`uhd-tx-s{k}`（送信，report，recovery），uhd-control（Action の booking，timed command の release，idle な stream の設定），uhd-clock（時刻の取り直し）．owner も設定を呼ぶ（configure，restore，flush）．
- **共有状態**：`device_gate`，`rx_chain`（stream ごとの `ClockMutex<RxChain>`），`tx_dsp_gates`，`tx_report_gates`，`tx_report_stash`，`rec`，`streams`，`held`，`rates`，atomic の `start_up` と `lost`．そのほか `ColdConfig` の updates．
- **lock の順**：文書は `updates → streams → rec`．実際には `updates → TX-DSP gate`，`RX-chain → streams`，`DSP gates → report gate → device gate` などが加わり，検査は `rec → streams` を見ない（`provider/core.rs:28-68`）．
- **呼び出しをまたいで持つ lock**：`ColdConfig.updates` は configure の apply の間，RX chain は start・restart・stop・flush の間，TX の gate は flush の 100 ms の間，native の control mutex は制御の呼び出しの間（棚卸し §2）．
- **block ごとの公開**：受信の block ごとに，chain の lock と `streams` の lock の下で cut を読み，届けた最初の sample（first-undelivered）を進める（`provider/rx.rs:681-717`）．Stop の booking はこの値を床に使う（`provider/core.rs:745-751`，RM-16）．
- **スレッド間のやり取り**：Action → control → TX（mpsc），control → RX の時刻つきの受け渡し（projection の lock と chain の lock），loss の公開（device gate → streams → atomic），holder（`Free`／`Owner`／`Release`／`FromNone`）と `reading`，streamer の generation と report の照合，segment と channel の move（棚卸し §3）．
- **テスト用の時計**：participant の登録，busy の間の停止，期限と notify の順は決められるが，二つの participant が同時に走れるときの順は決められない．標準の mutex は busy に数える．UHD の中のスレッドは participant ではない．idle の間は実時間で進む（棚卸し §8，`src/host.rs:8-20`）．
- **MockRadio**：`step(&mut self, until)` で Action と模型を一つの流れで進め，スレッドを作らない（`crates/ezsdr-mock-radio/src/lib.rs:73-95`）．判断を一か所で順番に行う形は，すでに Mock にある．

## 3. 制約

### 3.1 UHD 4.10 のスレッド安全性と，この Module の C の橋渡し

- C++ の streamer では，`send`，`recv`，`recv_async_msg` は別々のスレッドで走ってよい．一つの streamer への `send` どうし，`recv` どうしは呼ぶ側が直列にする．VERIFIED（[general.dox:222-246](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/driver_usage/general.dox#L222-L246)）．
- **ただしこの Module は C API を通して呼ぶ**．UHD 4.10 の [`usrp_c.cpp`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/usrp_c.cpp) は，同じ TX handle の `send` と `recv_async_msg` をどちらも `UHD_SAFE_C_SAVE_ERROR` で包み，[`error.h`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/include/uhd/error.h) はその handle が共有する `std::string last_error` を，成功のときも消して書く．よって一つの C handle への呼び出しを二つのスレッドから同時にすると，同期されない C++ の文字列への書き込みになる．VERIFIED（Sol，2026-10-31）．今は streamer ごとの call mutex がこれを防いでいる（`src/uhd.rs:1124-1150`）．**この設計では，一つの streamer の C handle を呼ぶのは，その stream の実行役一本だけにする**（§4.2）．
- 設定の変更は複数のスレッドから呼んでも壊れはしないが，結果の設定が不確かになりうるので，設定を触るのは一つのスレッドの文脈にするよう勧める．VERIFIED（general.dox，同じ所）．
- `set_command_time` → 設定 → `clear_command_time` は一つの操作ではない．command の queue が満ちると UHD は待つ．VERIFIED（[timed_cmd.dox:71-115](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/driver_usage/timed_cmd.dox#L71-L115)）．
- `recv` と `issue_stream_cmd` を別々のスレッドで同時に呼んでよいという明示の約束は見つからない．INFERRED．この設計では同じ実行役が呼ぶ．
- native の `apply` は一つの呼び出しの中で複数の操作と read-back を行う（`src/uhd.rs:903-990`）．途中まで効いて失敗することがある．

今の Module は，設定を uhd-control と各 owner の両方から呼ぶので，「設定は一つの文脈」の勧めから外れている（native の control mutex で直列にはしている）．

### 3.2 実時間の予算

- 受信の block は既定 2,000 sample で，200 MS/s なら 10 µs ごと（`profile.rs:14-15`）．送信の 1 回の send は最大 20,000 sample で，200 MS/s なら 100 µs（`provider/tx.rs:211-214`）．これは計算で，スケジューリングの保証ではない．
- timed release window 3 ms，start lead は受信・送信とも 50 ms，送信の in-flight 10 ms，device lead 2 ms，delivery allowance 3 ms（`crates/ezsdr-radio/src/x3xx.rs:397-402`）．
- 今の受信の道は，block ごとに chain の lock と `streams` の lock を取る（§2）．

よって判断役は block ごと，send ごとの道に乗れない．実行役は判断役から渡された計画と権限を手元に持ち，境目でだけ判断役に知らせる．速さと tail の遅れが足りるかは §7 で測る．

### 3.3 actor 案が二度退けられた理由

設計メモ 35 §5 は範囲の大きさで，44 §7 は技術的な理由で，stream ごとの actor を退けた．uhd-rx は `recv` で止まり，uhd-control の時刻つきの受け渡しがその後ろに並んで release window に間に合わない．この設計は**時刻つきの設定を受信の実行役ではなく設定の実行役が呼ぶ**ので，受け渡しは `recv` の後ろに並ばない．ただし受け渡しと受信の止まる時の順は，§4.4 の cell の手順で決める必要がある（Sol 1 回目 2）．`recv` の後ろに並ぶのは，その stream 自身の呼び出し（stream command，streamer を開く・閉じる，送信の flush）だけで，それは今も owner が自分で呼んでいる．

### 3.4 変えないもの

Kernel の契約（Provider の口，Stream Contract，event），MockRadio，Python，profile の値，UHD の FFI の安全の約束（一つの streamer の C handle への呼び出しを直列にする，閉じた streamer を呼ばない，loss の後に handle を手放さず残す）．Module の外から見える振る舞いは変えない．

## 4. 推奨：判断役，実行役，受け入れの cell

### 4.1 形

```
Kernel Action ─┐                         ┌─> 設定の実行役 uhd-settings（1 本）──> 設定，時刻つきの設定
完了・境目 ────┼─> 判断役 uhd-brain ─────┼─> stream の実行役 uhd-rx-s{k}（stream ごと）──> open/close, recv, stream command
report ────────┤   step(state, input)    ├─> stream の実行役 uhd-tx-s{k}（stream ごと）──> open/close, send, recv_async_msg, flush
時刻・loss ────┘   -> commands           └─> 記録，timer の予約
                        │
                        └── stream ごとの受け入れの cell（判断役・その stream の実行役・設定の実行役だけが µs の間触る）
```

### 4.2 判断役と実行役

- **判断役**（`uhd-brain`，instance に一つ）：一つの値 `Brain` が判断に要る状態を持つ：timeline（今の `Streams`），stream ごとの `RxChain`／`TxChain`，held の予約，recovery，streamer の generation と report の照合，予約した照合（期待する ACK），書く記録，実行中の command．遷移は `fn step(&mut self, input: Input) -> Vec<Command>` だけ．装置を呼ばず，時計を読まない（時刻は input に入る）．一本のスレッドで input を一つずつ処理する．
- **input**：受け付けた Kernel の Action，timer の満了，実行役の完了（`Done { cmd, outcome }`），境目（最初の sample，cut で止まった，DDC の値を消費した，overflow，underflow），report（ACK，underflow，time error．読んだ streamer の generation つき），loss，stop と shutdown，時刻の取り直し．
- **command**：設定の実行役へ（値の適用，時刻つきの値の適用），stream の実行役へ（計画と権限の差し替え，streamer を開く・閉じる，stream の開始・停止，送る burst，flush），記録，timer の予約．どの command も `(generation, id)` を持つ．
- **設定の実行役**（`uhd-settings`，一本）：設定の command を届いた順に一つずつ実行する．`set_command_time` から `clear_command_time` までは一つの command の中で行う．streamer の handle には触らない．UHD の command queue が満ちると待つ（今も native の control mutex の下で同じく待つ）．
- **stream の実行役**（`uhd-rx-s{k}`，`uhd-tx-s{k}`）：その stream の streamer の C handle を呼ぶ唯一のスレッド（§3.1）．streamer を開く・閉じる，`recv`／`send`，stream command，送信の report の読み（`recv_async_msg`），送信の flush．report には読んだ streamer の generation を読んだ時点で付ける（Sol 1 回目 5）．flush は判断役から見て非同期の手順で，「送った」「ACK を読んだ」「期限が来た」を別々の event で返す．flush の 100 ms の待ちは stream の実行役が持ち，設定の実行役の列を止めない．
- **uhd-clock** は今どおり時刻を取り直し，結果を判断役への input として送る．

### 4.3 実行役が自分で決めること

実行役は「判断しない」のではなく，**判断役が渡した計画と権限の範囲で，決まった純粋な規則で決める**（Sol 1 回目 4）．その規則は判断役と同じ crate の純粋な関数で，DST からも呼ぶ．

- 受信：lattice の丸め，重なりの切り落とし，gap の由来，cut での切り詰め，DDC の値の消費（受け入れた時点での資格，資格のある sample ごとに一つ，同じ時刻の値には別々の sample．設計メモ 39 §6.8），期限の確認．
- 送信：部分的な send の cursor，期限，保持した最後の sample の扱い．

判断役が決めるのは，計画と権限そのもの（どの segment を，どこまで，どの cut で，どの DDC の値を持って流すか）と，境目を受けた後のこと．

### 4.4 命令の状態

判断役は，装置に効く命令の一つ一つを，次の状態で持つ（Sol 2 回目 1）．

| 状態 | 意味 | 取り消せるか |
|---|---|---|
| 列にある | 判断役が出し，実行役がまだ取っていない | 判断役が取り消せる（実行役は取る前に cell で確かめる） |
| 取った | 実行役が cell で確かめて取り，装置の呼び出しを始める前 | もう取り消せない．呼び出しは始まるものとみなす |
| 効いたかもしれない | 装置の呼び出しが始まった（UHD の中で待っているかもしれない） | 取り消せない．結果は分からない |
| 終わった | 呼び出しが戻った．結果は成功，失敗，途中まで効いた失敗（native の `apply` は周波数を変えた後に command time の解除や read-back で失敗しうる，`src/uhd.rs:930-968`）のどれか | — |
| 始末した | 判断役がその結果を一度だけ記録と chain の状態に反映した | — |

途中まで効いた失敗は「装置に値が残ったかもしれない」として扱い，失敗とは数えない（今の `left` と同じく，その chain の後の使い方に効く）．判断役は「取った」「効いたかもしれない」の命令がある chain を**ピン留め**し，それが「終わった」になるまで，その chain の restore，channel の移し，streamer の閉じる・開く，次の segment の開始を出さない．受信を止めることは止めない（ただし §4.6 の native の lock で待つことがある）．

### 4.5 受け入れの cell と四つの手順

stream ごとに一つの小さな値を `Mutex` で持つ．触るのは判断役，その stream の実行役，設定の実行役で，どれも装置の呼び出しをまたいで持たず，中でする仕事は上限のある量（受け渡し中の値の数に上限を置く）に限る．中身：

- chain の相（`Off`／`Armed(segment)`／`Streaming(segment)`／`Stopping(segment)`），その segment の channel と generation．`Armed` と `Streaming` は実行役が開始の確認とともに書く．`Stopping` は受け入れを閉じた相で，そこから `Armed` に戻るのは，判断役が出した開始を実行役が確認したときだけ．
- その retiring epoch の最新の cut（Stop が短くした後の値）．
- 計画の版（epoch）と，実行役が自分で止まってよい点（計画の cut）．
- 実行役が進めた所（届けた最初の sample．今の first-undelivered）．
- 受け渡し中の DDC の値の列（値ごとに §4.4 の状態）．

**手順 1：時刻つきの DDC の値**（線形化点は cell の中の予約）．判断役は cell の中で，相が `Armed(s)` か `Streaming(s)` であること，値の channel が s のものであること，generation が一致すること，値の時刻が s の origin 以上で計画の cut より前であること，実行役が進めた所が計画の cut より前であることを確かめて，値を「列にある」で cell に置き，設定の実行役へ command を出す（今の `eligible`，`src/provider/core.rs:152-156` と同じ条件に，generation と cut を加える）．設定の実行役は cell の中で，その値がまだ「列にある」かを確かめて「取った」にし（取り消されていれば呼ばずに戻る），cell を離して呼び，戻ったら cell に結果を書く．受信の実行役は，資格を受け入れた時点で判定して値を消費し（設計メモ 39 §6.8），計画の cut で止まる前に，「列にある」値は取り消し，「取った」「効いたかもしれない」値がある間は drain を続ける．

**止まると決めるとき**（受け入れを閉じる遷移．Sol 3 回目 1）：受信の実行役が止まると決めるのは cell の中の一つの遷移で，相を `Stopping(s)` にし，「列にある」値を取り消し，その時点で「取った」「効いたかもしれない」値を残りの義務として記す．`Stopping` の間，判断役の予約は成らない（予約の条件は `Armed` か `Streaming`）．よって止まると決めた後に入る予約はない．判断役が出す閉じる・差し替え・restart も，同じ遷移で受け入れを閉じてから行う．今のコードは chain の lock を stop の判断と呼び出しの間持つことでこの順を保っている（`src/provider/rx.rs:271-313`，`control.rs:850-881`）．

止まると決める前から「取った」値が drain の期限を過ぎて終わる場合（設定が UHD の queue で止まった）は，受信を止めた後に値が効きうる．その値はピン留めのまま `left` に残り，次の segment が照らす．これは今の L1 より一歩進んだ扱いで，UR-24（`design/18-uhd-radio.md:274-276`）の drain と止まり方の文を M1 で改める（spec の改めとして記録する）．drain の期限を過ぎたら受信を止め，残りの値を報告するが，**値は取り消さずピン留めのまま残す**．遅れて「終わった」になったら，判断役がそれを一度だけ始末する（止まった chain に効いた値は `left` として残し，次の segment の consume が照らす．今の L1 と同じ）．

**手順 2：Stop と cut の変更**．判断役の Stop の booking は cell の中で，実行役が進めた所を読み，計画の cut を書き換える．新しい cut 以後の時刻を持つ値は，「列にある」なら取り消し，「取った」以後なら手順 1 の drain とピン留めに従う．よって Stop の cut は公開済みの sample より前にならず，実行役は新しい cut を越えて公開しない．

**手順 3：計画の取り消しと静止の確認**．streamer の差し替え，channel の移し，restart では，判断役は cell の epoch を進め，その chain のピン留めが外れ（設定の実行役の命令が終わった），かつ stream の実行役が「今の呼び出しから戻り，新しい epoch を読んだ」と cell に書くまで，その streamer や chain に依る命令を出さない．新しい epoch を読む前の `recv` が返した buffer は，**retiring epoch の最新の cut**（Stop が短くしたならその値．`recv` を始めたときの計画の cut ではない）までだけ公開し，その先は数えて捨てる（今の `rx_before_origin` と同じ扱い）．buffer の公開と届けた所の前進は cell の一つの区間で行い，静止の確認はその後に書く（Sol 3 回目 4）．

**手順 4：失敗（loss）と判断役の panic**．instance に一つの失敗の印（atomic）を置く．loss を知った者は，記録より先に印を立て，その後 native の `mark_lost` を呼ぶ．どの実行役も，装置の呼び出しの前と，自分で回す loop の一回ごとに印を確かめ，立っていれば呼ばずに列の残りを捨てる．印を確かめてから呼び出しが始まるまでの窓は残る（今も同じ）．streamer を開く・閉じる・rate を読むことは，今の `device_gate`（`src/provider/core.rs:1314-1323,1560-1607`）を残して loss と直列にする（今と同じ強さ）．判断役が panic したら，その unwind の guard が失敗の印を立て，native の `mark_lost` も呼ぶ（native の destructor は別の loss の印を見て handle を残すかを決める，`src/uhd.rs:342-365,1174-1175`．判断役が自分の loss の input を処理することに頼らない）．loss の後も streamer の handle は手放さず残す（Sol 1 回目 3，3 回目の note）．

**cell の中でする仕事**（Sol 3 回目 5）：cell の中では，前もって計算した結果の確かめと書き込みだけを行う．sample を走査する消費の判定，切り詰め，割り当て，メッセージの送り，装置の呼び出しは cell の外で，cell から写した計画（epoch と cut つき）を元に計算し，書き込むときに epoch と cut が変わっていないかを確かめ，変わっていれば計算し直す．受け渡し中の値の数は chain ごとに上限を置き，上限に達したら判断役は値を予約せず held の列に残す（今の held の列と同じ扱いで，新しい見える Ceiling にはしない）．cell を持つ時間と待つ時間の分布を §7 で測る．

### 4.6 同期の数え方（受信の一切れ，Sol 2 回目 6 を元に）

| 種類 | 今 | この形 |
|---|---|---|
| 意味の上の状態の持ち主 | uhd-control，各 owner，`Streams`，`RxChain` が分け持つ | 判断役ひとり |
| 装置の呼び出しをまたいで持つ意味の lock | `ColdConfig.updates`，RX chain，TX の gate | なし |
| block の公開 | chain と `streams` の二つの lock | 写しを取る区間と，確かめて書く区間の cell 二つ（書き込みが無効になれば取り直し）．前の書き込みで次の写しを渡す工夫をすれば一つ |
| 時刻つきの受け渡し | projection の lock，chain，timeline の読み | cell の三区間（予約，取る，結果）と命令・完了のやり取り |
| lifecycle と loss | `device_gate`，`lost` | `device_gate`，失敗の印（同じ） |
| native の lock（control，streamer の表，呼び出しごと） | ある | 同じく残る |

block の公開の取得の数は減らない（Sol 4 回目 2）．装置の呼び出しをまたぐ意味の lock は消え，sample の大きさの仕事が lock の中から出る．時刻つきの受け渡しは区間が増え，命令と完了のやり取りになる．native の lock は変わらない．得るものは数の減りより，**順番が判断役の状態として明示され，DST で探索できる**ことである．

### 4.7 native の lock

設定の実行役が UHD の中で待つ間，native の control lock を持っている（`src/uhd.rs:903-906`）．受信の stop（`:601-616`），streamer を開くこと，時刻の読み（`:872-876`，`:994-1000`）も同じ lock を要る．よって設定が UHD の queue で止まれば，受信の stop も待つ．今も同じである．この形は，その待ちを消さない．DST は native の lock を共有の資源として模型に入れ，取る・待つ・離すを出来事にする．drain や shutdown が期限内に終わることは，native の lock が空いている場合についてだけ約束する．空かない場合の結果（期限を過ぎて handle を残す）は今と同じに書く．uhd-clock は判断役へ input を送るだけで cell には触らないが，native の lock を取る者として模型に入る．cleanup は「すべての実行役が止まった」か「handle を残す」のどちらかでだけ進む．

### 4.8 速さ

判断役は block ごと，send ごとの道に乗らない．実行役は block ごとに cell を短く取る（今は二つの lock）．判断役に届く input は境目，Action，完了，timer，report で，一秒に数百程度と見積もる（INFERRED）．判断役の処理は止まらない（装置も legacy の lock も待たない）ので，一つの input にかける時間は短い見込み（INFERRED）．どちらも §7 で，受け渡しの手順，列の待ち，native の lock の取り合いを含めた端から端までの遅れと throughput として測る．

### 4.9 寿命

`Provider::stop` は判断役への input になり，判断役が各実行役に終わりの command を出す．join の上限（1 s），detach したときに streamer を残す扱いは今どおり．#67（遠い origin の timed start が queue に残ったまま stop が来て join を越える）は，装置の中に残る命令と host の join の方針の両方に懸かり，この設計だけでは消えない．

### 4.10 8b との関係

判断役は instance に一つ．8b で一つの instance が二台の装置を持つとき，台をまたぐ判断（PPS で揃えた開始など）も一か所で順番に行える．台ごとの実行役は増えるが，判断役は増えない．

### 4.11 MockRadio との関係

Mock はすでに一つの流れで進む模型で，判断役は同じ形になる．同じ Action の列を Mock と「判断役＋実行役の模型」に流し，記録を比べる差分テストが，スレッドなしで書けるようになる（今の `ur_33_d9_raw_probes` の差分の仕組みを広げる）．Mock と UHD の engine を一つにはしない（改善案 #5．Sol：parity が自分との比較になる）．

## 5. テストの組み立て

| 層 | 何を | どう |
|---|---|---|
| 0 | 判断役の遷移，実行役の純粋な規則 | `step` と規則の関数への表形式のテスト．純粋なコードなので自動生成の mutant（cargo-mutants）がよく効く．意味のある mutant は手書きの行で残す |
| 1 | 手順と順番（DST） | 判断役，cell，実行役の模型，装置の模型を一つのスレッドで動かす．「受け入れ」「装置への効果」「完了」「判断役への到着」を別々の出来事にし，実行役ごとの FIFO と因果（効果は受け入れの後，完了は効果の後）を守ったまま，次にどれを起こすかを選ぶ．小さな場合は上限つきで全部の順を試し，大きな場合は seed で選ぶ．一歩ごとに安全の不変条件を，終わりに進む性質（drain が期限内に終わる，始末がちょうど一回，shutdown が終わる，説明のない deadlock がない）を確かめる．失敗は trace で再現して縮める |
| 2 | 実行役と装置 | 薄い実行役を FakeDevice で，装置の模型を実機との conformance（改善案 #6）で確かめる．DST の装置の模型と本物のスレッドで動く FakeDevice の結果も比べる |
| 3 | 配線 | 本物のスレッドで動かす FakeDevice の smoke を少数 |

**装置の模型**：今の FakeDevice はそのままではスレッドなしに呼べない．時刻つきの apply は queue の空きを待って回り（`src/device.rs:1476-1518`），受信は packet ごとに待ち（`:1786-1843`），report の読みは期限まで眠る（`:2091-2148`）．DST には，仮想の時刻で進み，待つ代わりに「まだ」を返して後で再開できる形の装置の操作が要る．UHD の queue が満ちたときの待ち，部分的な受け入れ，packet の到着，sample で進む DSP の消費，report の遅れ，装置の時刻の進みを含む．これを FakeDevice から切り出し，本物のスレッドの FakeDevice もそれを使う形にする（模型が二つにならないように）．

**不変条件**の元は spec 18 の UR と設計メモ 44 §3.6．例：失敗の印が見えた後に**受け入れられる**装置の呼び出しがない（command を出さないことではなく，実行役の受け入れを見る．確かめてから呼ぶまでの窓は今と同じく残るので，不変条件はそれを含めて書く．Sol 1 回目 3，2 回目 3）．時刻つきの DDC の値は，cell の予約の時点で相・channel・generation・origin・cut の条件を満たしたものだけが「取った」になる．ピン留めの chain に restore，移し，閉じる・開く，次の segment の開始が出ない．渡した値は消費されるか報告される．Stop の cut は公開済みの sample より前にならない．SOB と EOB が対になる．閉じた streamer の handle を呼ばない．recovery の前に payload を送らない．記録の行が重ならない．

今の約 1,000 本のテストは，Module の外の口（Provider と FakeDevice）を通るものが多い．移行の間はそのまま回帰の網に使う．中の仕組みに触れるテストは adapter で残し，外から見える assertion と mutation の行を保つ．消すのは，名前のついた代わりが同じ失敗を捉えると示せたときだけ（AGENTS.md §9 Maintainability の 3）．一般の不変条件があるだけでは，payload，timestamp，配線の検査が残る証拠にならない（Sol 1 回目 7）．

## 6. spec と実装の境目

spec 18 の UR は，外から見える契約（Kernel と実験者に見える event，時間の保証，Ceiling，拒否）と，安全の約束（§3.4）を書く．スレッドの名前，lock，claim，遷移表は spec から出て，コード（判断役の遷移と cell の手順）と crate の非規範の設計文書に移る．規則の ID とテストの義務は残し，改めは AGENTS.md §2 の形で記録する．規範の文を，別々に手入れする文・遷移表・不変条件の三つに書き直して二重にはしない．不変条件は spec の観測できる規則をテストに結ぶ．この移しは移行の各段階の中で，その段階が触る UR についてだけ行う．

## 7. 実験：書き直す前に証拠を取る

**確かめたいこと**：

- C1：判断役，cell，実行役の形で，一番難しい一切れを書け，同期は §4.6 の表の「この形」の列に限られ，速さと tail の遅れが今より悪くならない．
- C2：DST が，その一切れの手順の誤りを，決まった予算の中で見つける．
- C3：§1 の仮説：その一切れの過去の不具合の多くが，この形では起こりえないか，DST で見つかる．

**一切れ**：受信の chain と uhd-control の受け渡し，loss，stop，streamer の閉じる・開く（L1 と D9b-2 の範囲）．競合の履歴が一番多い．

**作るもの**（本番の道にはつながない．採らないと決めたら消せる）：

1. その一切れの判断役と cell の手順（segment の開始，最初の sample，cut，stop，時刻つきの DDC の値の予約・適用・消費・drain・flush，restore，loss，close／open，channel の move）．
2. 再開できる装置の模型（§5）を FakeDevice の受信と DDC の部分から切り出したもの（受信は command queue，front end ごとの DSP，送信に依る sample の生成と装置の状態を共有している，`src/device.rs:772-797,1851-1859`．待ちを「まだ」に替えるだけでは済まない），native の lock の模型，実行役の模型，DST の harness（上限つきで全部の順を試す方法と seed，進む性質，trace の再現と縮め）．本物のスレッドの FakeDevice も，切り出した模型に装置の意味を任せる．
3. 本物のスレッドで動く薄い実行役と，それを FakeDevice の上で動かす adapter（DST の結果と比べる）．
4. 速さの測り方：200 MS/s 相当の block の率で，受け渡しの手順と列の待ちを含む端から端までの遅れと throughput を，今の owner と比べる．

**成功の条件**（実装の前に固める．owner に決めてもらう案．Sol 1 回目 6，2 回目 7）：

- **過去の不具合との対応表**：D9b-2 の 6 件，L1 の M1・M3・受信の窓・配線し忘れを一つずつ，「この形では起こりえない（理由）」「DST が見つけた」「解けていない」に分ける．解けていないものが残れば理由を書く．
- **独立に選んだ誤り**：実装の前に，Sol が設計メモ（コードではなく）から，判断役と cell の手順に入れる振る舞いの誤りを 20 件選んで固める（oracle を止めるだけのものは除く）．外から見える assertion を変えずに，DST が 16 件以上を見つける．
- **小さな場合の全探索**（Sol 3 回目 3：出来事 60 個は 10¹⁷ 通りを超えて終わらない）：実装の前に，次の場合の出来事の列と粒度（受け入れ，効果，完了，到着，native の lock の取る・離す）を固め，Sol の計画レビューで承認を得る．どれも実行役ごとの FIFO を守ったうえで，すべての順を試す．一つの場合は 1 コアで 10 分以内．探索した状態の数と順の数を記録し，打ち切った探索は失敗に数える．
  1. 値 1 つ，受信が cut で止まる，Stop が cut を短くする．
  2. 値 1 つ，loss が受け入れ・効果・完了のそれぞれの前後に入る．
  3. 計画の取り消し，`recv` からの戻り，Stop（retiring epoch の cut）．
  4. 同じ時刻の値 2 つと，止まると決める遷移．
  5. 設定が UHD の queue で止まったまま drain の期限が来る（native の lock を含む）．
  6. 判断役の panic と，実行役の自分で回す loop．
  順を減らす工夫（partial-order reduction）を使うなら，安全と進む性質を保つ理由とともに前もって書き，減らした数を記録する．
- **seed の探索**：上の場合を広げたもの（stream 2 本，値 3 つ，Stop と差し替えと loss を混ぜる）を sim02 で 30 分，または seed 10,000 個．時間切れは失敗に数える．
- **同期**：§4.6 の表の「この形」の列のとおりで，装置の呼び出しをまたいで持つ意味の lock も，本体の probe もない．
- **速さ**：FakeDevice の上で，受信 200 MS/s 相当の block の率と 1 MS/s の両方で，時刻つきの受け渡しの release から装置への適用までの遅れの p50・p99・p99.9 と，block の取りこぼしを，今の owner と比べる．p99 の遅れが今より 100 µs 以上悪くならず，release window の 10 %（300 µs）以内．取りこぼしは今以下．更新は数と時刻の表を前もって固めて流し，**流した更新はすべて勘定する**：予期しない拒否，取り消し，適用されなかった更新は速さの判定の失敗に数える（遅い更新が抜けて percentile が良く見えることを防ぐ）．観測の時間と，p99.9 を出す標本の数を記録する．故障の場合の結果は別に試す（Sol 4 回目 3）．cell を取り直した回数も遅れに含めて測る．
- **再現**：DST の失敗は trace で再現し縮められる．本物のスレッドの adapter は，test build の中で受け入れ・効果・完了の順を強制する仕組みで同じ trace を再現する．二つの包み（DST とスレッド）が同じ装置の模型に任せることで，包みの正しさを確かめる（装置の忠実さは別に conformance で確かめる）．

**失敗したら**：今の形を続け，次の二つを代わりに考える：判断を純粋な関数として今のスレッドの中で取り出す（§9 の二つ目），今のコードの同期の部品を差し替えて探索する（§9 の三つ目）．

**大きさ**：L1 より大きい．本体とテストで 2,500〜4,000 行は楽観の見込みで，範囲の上限ではない（Sol 2 回目 7）．risky の手順（系列の表 → テスト計画 → Sol の計画レビュー → 実装 → Sol のコードレビュー）で進める．

## 8. 移行の段階（実験が成功したら）

どの段階でも，状態の書き手はひとりで，その直列の境目を書いておく（Sol 1 回目 7）．

1. **M1 timeline，booking と release，受信**：判断役が `Streams`（timeline），uhd-control の booking と release，受信の chain の唯一の書き手になる（今の cold booking は sequence の割り当て，projection と coercion，move の booking，設定の公開を一つの区間で行う，`src/provider/control.rs:999-1058`．これを分けないために booking と release を M1 に入れる．Sol 2 回目 5）．受信の実行役，設定の実行役，受信の cell．送信の owner は，`Streams` を判断役への**取引ごとの**問い合わせ（一つの問い合わせが今の一つの lock の区間に当たる）で読み書きする．判断役の処理は止まらず，legacy の lock を取らず，実行役の完了を待たない．よって送信の owner が legacy の lock（TX の gate，`held`）を持ったまま判断役を待っても，待ちの輪はできない．問い合わせの一覧と，それぞれの呼び出し側が持ってよい lock を，M1 の brief に書く．

   **projection と設定の受け入れ**（Sol 3 回目 2）：今の `ColdConfig` の projection（booking が `updates` の下で書き，送信の設定が `updates` を持ったまま装置を設定する，`src/provider/control.rs:142-184,678-683,917-933`）は M1 で判断役のものにする．判断役は projection を版つきの変わらない写し（`Arc`）として持ち，booking と取り消しで新しい版を作る．stream を設定するときは，判断役が「版 v での設定を許す」を出し（受信は判断役から設定の実行役へ，M1 の送信の owner は問い合わせ `設定を許可して(stream, v)` → `許可` か `古い` で），設定した者が `設定を終えた(stream, v, 結果)` を返す．設定の間に新しい版ができたら，判断役は終わりを受けた後にもう一度設定を許す（今の `configured` の印と `updates` の lock がしている順を，版の比較に替える）．判断役は装置も `updates` も待たない．よって M1 で `ColdConfig.updates` の lock は要らなくなる（M3 を待たない）．

   ただし版の比較は古い設定を見つけるだけで，装置に効く順は決めない（Sol 4 回目 1：版 v で設定を許された送信の owner が止まっている間に，v+1 の時刻つきの値が release されて当たり，その後で古い設定が上書きする．projection は segment の origin までの更新しか含まないので，やり直しでも origin より後の値を上書きしうる）．そこで**設定の許可は実行中の操作**として判断役の状態に置き，終わるまで，同じ channel にぶつかる release と lifecycle の命令を判断役が**後回しにする**（判断役は待たない．後回しにした命令は，設定の終わりを受けてから出す）．許可の取り消しはなく，途中まで効いた失敗は §4.4 と同じく「装置に値が残ったかもしれない」として扱う．これは M1 の設計の中で固め，M1 の前に解く（受信の実験は，設定の実行役が設定の命令を丸ごと直列にするので，これに縛られない）．M1 の問い合わせは：timeline の取引（今の `streams` の区間ごとに一つ），projection の写しを読む，設定を許可して，設定を終えた，held の予約と解放．送信の owner はこれらを TX の gate（M2 まで残る）を持ったまま呼んでよい．判断役はどの legacy の lock も取らないので，待ちの輪はできない．
2. **M2 送信**：判断役が送信の chain と report の照合の唯一の書き手になる．送信の実行役（send，report の読み，flush），送信の cell．TX の gate と stash を消す．
3. **M3 片付け**：adapter，holder，`reading`，probe，lock の順の検査を消す（`device_gate` は §4.5 の手順 4 のため残す）．触った UR を §6 の形にする．
4. **8b の準備**：一つの判断役が二台を持つ．

各段階は AGENTS.md §9 の risky の手順で，段階ごとに tree を緑に保つ．M1 と M2 の後に実機で確かめる．

**D9b-1b と #75**（owner 2026-10-31）：実験の後に置く．Sol は「未解決の不具合を，まだ証明されていない移行のために遅らせない」と勧めた（1 回目 7）が，owner は急がないとし，#71・#75 はどちらも装置の異常か特殊な使い方でだけ出る．しかも #75 の問い（確認の取れない失敗を普通の値に潰さず型で表す）には §4.4 の命令の状態がほぼ答える．D9b-1b を先にすると，送信の recovery と report を M2 でもう一度書き，同じ問いに二つの答えを作る．
- 実験が成功したら：送信の設計（M2）をこのメモに足し，その中で D9b-1b の中身（R2′，(a)）と #71・#75 に答える．
- 実験が失敗したら：#75 の設計メモを書き，D9b-1b を今のコードで行う（それまでの予定どおり）．

欠点：#71・#75 の修正が遅れる（成功した場合は M1・M2 まで）．その間は issue のまま既知として残す．

**順番**（owner 2026-10-31：急がない，並行にしない）：トークン照合 → rustfmt → 改善案 A（gate の 1 コマンド化，記録を repo に，指示を 1 か所に，major は反例で）→ 実機 → この実験 → 結果で分かれる（上）→ 8b の前に移行を終える．Phase 8 の終わりの振り返りのうち，純粋な型に遷移のテストを寄せる案と，操作の列を生成するテストの案はこの実験で答え，振り返りは mutant の自動生成とテストの準備の共通化に絞る．

## 9. 採らない案

- **今の形のまま型を足し続ける（35・44 の延長）**：判断の場所が散ったまま並行に走り，lock は装置の呼び出しをまたぐ．スレッドの順番を OS が決めるので，手順を DST にかけられない．実験が失敗したときの戻り先として残す．
- **判断を純粋な関数として今のスレッドの中で取り出す（Sol 1 回目 8）**：遷移は表で試せるようになるが，どの関数をどのスレッドがどの lock の下で呼ぶかの順は OS に任されたまま残り，探索できない．移行の手間は小さい．実験が失敗したときの第一の代わり．
- **今のコードの同期の部品を差し替えて探索する（shuttle や loom の型，改善案 #1(c)）**：`ClockMutex`，標準の mutex，mpsc，condvar，`HostClock`，FakeDevice の mutex を模型の部品に替える必要があり，UHD の中のスレッドは模型にできない（棚卸し §8）．置き換える予定のコードを探索することになる．実験が失敗したときの第二の代わり．
- **stream ごとの actor（設計メモ 35・44 で退けた形）**：`recv` で止まる間，時刻つきの受け渡しが待たされる（§3.3）．
- **送信の report を別のスレッドで読む（第 1 版）**：今の C の橋渡しでは同じ handle の `last_error` を共有するので安全でない（§3.1）．C++ の小さな橋渡しを自分で書いて呼び出しごとに誤りを受け取れば可能だが，FFI の変更になり，得るものは send と report の並行だけ．
- **async の runtime（tokio など）**：依存が増え，共有状態での判断は残る．UHD の呼び出しは止まる FFI なので，結局スレッドで包む．
- **Mock と UHD の engine を一つにする（改善案 #5）**：parity が自分との比較になる．
- **装置ごとに判断役を置く**：8b で台をまたぐ判断に，判断役どうしのやり取りが要る．

## 10. owner に決めてもらうこと（一つずつ）

1. **方向と実験**：§4 の形を目指し，まず §7 の実験を，§7 の成功の条件で行う（推奨）．代わり：今の形を続ける，または判断を純粋な関数として今のスレッドの中で取り出す（§9）． → **owner 2026-10-31：実験を進めてよい．急がないので，他の作業と並行にはせず順番に行う**．
2. **実験の一切れ**：受信の chain と受け渡し（推奨．競合の履歴が一番多い）．代わり：送信の recovery と report．
3. **D9b-1b**：→ **owner 2026-10-31：実験の後に置き，結果で分ける**（§8）．

## 11. レビュー

Sol 6.1，1 回目（2026-10-31）：REVISE．方向は有望だが，移行の承認ではなく，見直した使い捨ての実験を．major 7・minor 1．Sol 自身の答え（§4 を読む前）：寿命と装置の操作の受け入れを，止まらない，スレッドのない一つの状態機械に置き，実行中の操作と完了を明示する．sample の移動は実行役に残す．スレッドをまたぐ「確かめてから呼ぶ」約束を，強制できる順序に替える．
(1) 送信の report を別スレッドで読むのは今の C の橋渡しでは危ない（同じ handle の `last_error`）→ §3.1，§4.2，§9．
(2) 実行中の command の記録だけでは chain の lock の代わりにならない（受信が cut で止まった後に設定の実行役が値を当てる）→ §4.4 の cell と手順 1・3．
(3)「loss の後に command を出さない」は今の安全の約束より弱い（列に残った command，実行役が自分で回す loop，判断役の panic）→ §4.4 の手順 4，§5 の不変条件．
(4) block ごとの公開の境目が抜けている．実行役が決めることを明示する → §4.3，§4.4 の手順 2．
(5) 送信の flush，report の generation，設定の列の待ち → §4.2（flush は stream の実行役が非同期の手順で，report は読んだ時点の generation で）．
(6) 実験が形を確かめずに通りうる．FakeDevice はスレッドなしでは動かない．成功の条件の作り直し → §5 の装置の模型，§7．
(7) M1〜M3 の途中で書き手が二人になる．D9b-1b は今のコードで → §8．
minor (8) 一つの根は仮説，「DST にできない」などの言い過ぎ，#67 の扱い，純粋な判断を先に取り出す案の欠落 → §1，§4.6，§9．
note：§6 の spec の方向は妥当．規則の ID とテストの義務を残し，規範の文を三つに二重化しない → §6．

Sol 6.1，2 回目（同日）：REVISE．1 回目の (1)(4) は閉じ，(5) はほぼ閉じ，(6) は改善，(2)(3)(7) は残る．major 5・minor 2．結論：得る一番の利点は同期の消滅ではなく，明示され探索できる操作の順番．
(1) 確かめた予約が drain を越えて，止まった・差し替えた chain に効く（設定が UHD の queue で止まる）．途中まで効いた失敗 → §4.4 の命令の状態とピン留め，§4.5 の手順 1．
(2) 予約の条件が今より弱い（相，channel，origin），Stop が cut を短くしたときの扱いがない，古い epoch の buffer → §4.5 の手順 1〜3．
(3) loss の窓と §5 の不変条件の食い違い，stream ごとの印では instance の失敗にならない，判断役の panic → §4.5 の手順 4，§5 の不変条件．
(4) native の control lock が模型の外にあり，期限内の進みを壊しうる →  §4.7．「cell だけ」の主張をやめた（§1，§4.6）．
(5) M1 の adapter は取引の区間を壊しうる，待ちの輪 → §8 の M1（booking と release を M1 に，取引ごとの問い合わせ，判断役は止まらない）．
minor (6) 同期の数を正しく数える → §4.6 の表．(7) 成功の条件の数値，装置の模型の切り出しの難しさ，行数は楽観 → §7．

Sol 6.1，3 回目（同日）：REVISE．方向は「明示され探索できる操作の順番」としてかなり明確になった．native の lock の模型とピン留めで前の穴の大事な所は閉じた．major 3・minor 2．
(1) 受信が自分で止まると決めるときに受け入れを閉じる遷移がなく，止まると決めた後に予約が入りうる → §4.5 の `Stopping` の相と「止まると決めるとき」．期限を過ぎて終わる値の扱いは UR-24 の改めとして記録する．
(2) M1 で legacy の projection の持ち主が決まっていない（判断役が止まらない前提が成り立たない）→ §8 の M1 の「projection と設定の受け入れ」（版つきの写し，設定の許可と終わり，`updates` の lock は M1 で消える）．
(3) 出来事 60 個の全探索は現実的でない → §7 の小さな場合の全探索（6 つの場合を前もって固める）と seed の探索．
minor (4) 古い epoch の buffer に使う cut があいまい → §4.5 の手順 3（retiring epoch の最新の cut，公開と前進は一つの区間）．(5)「上限のある仕事」では µs を保証しない → §4.5 の「cell の中でする仕事」．
note：判断役の panic の道でも native の handle を残す → §4.5 の手順 4．

Sol 6.1，4 回目（同日）：ADOPT WITH CHANGES．owner が §10 の 1 を方向と使い捨ての実験として決められる状態．移行の承認ではない．major 1・minor 2．
(1) M1 の版の比較は古い設定を見つけるが装置に効く順を決めない → §8 の M1（設定の許可を実行中の操作にし，ぶつかる release を後回しにする．M1 の前に解く）．
minor (2) block の公開は写しと書き込みで cell 二つになり，2 → 1 の主張は成り立たない → §4.6 の表と文，§7 の C1．(3) 速さの判定で流した更新をすべて勘定する → §7 の速さ．
note：Sol の見立て ― 得られるのは明示された操作の状態と探索できる順番であって，同期の総数の減りは示されていない．意味の lock が装置の呼び出しをまたがなくなることと，判断が一か所になることは実質的な変化．行数の見込み 2,500〜4,000 は楽観．
