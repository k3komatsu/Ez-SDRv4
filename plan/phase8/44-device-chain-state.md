# 設計メモ 44：装置の chain の状態を，持ち主のある値にする

Status: 第 6 版（2026-10-30，orchestrator Opus）．Sol のレビュー 6 回（5 回 REVISE，6 回目 ADOPT WITH CHANGES．変更は入れた）．owner の決め（2026-10-30）：範囲を絞らず，最終形のきれいな方を採る．時刻つきの DDC の値は，その segment を流している streamer にだけ渡し，そうでない間は uhd-control が持ち続ける（§3.1）．経過は §10．**L1（§3，受信）は実装済み**（2026-10-31，[00-overview](00-overview.md) の L1 行，[reviews/l1-sol.md](reviews/l1-sol.md)）．L2（§4，送信）は未着手．

| 項目 | 内容 |
|---|---|
| きっかけ | AGENTS.md §6 の三振．D9b-2 だけで `mech:stream-lifecycle-scattered` が 6 件（[reviews/d9b2-sol.md](reviews/d9b2-sol.md)），その前の D9b-1a で `mech:device-state-outlives-call` が 6 件（[reviews/d9b1a-sol.md](reviews/d9b1a-sol.md)）．owner 2026-10-30：D9b-1b の前に見直す．コストを惜しまない |
| 材料 | 設計メモ [35](35-uhd-stream-state.md)（`Streams` の holder と段階），[39 §6](39-dsp-timed-commands.md)（T1〜T3，R2′），[42](42-d9-production-fix.md)（D9 の R1），[43](43-refusal-keeps-origins.md)，main `22adbb7` の `crates/ezsdr-radio-uhd/src/provider/{rx,tx,core,control,mod}.rs`，[failure-mechanisms](../maintenance/failure-mechanisms.md)，#67 |
| 機構 | 二つの機構が同じ所に集まっている：`device-state-outlives-call`（装置は chain ごとに状態を持ち越す）と `stream-lifecycle-scattered`（stream の寿命が別々の印に散る） |
| risky | はい：寿命と，owner と uhd-control の受け渡しの順序（AGENTS.md §9）．見える振る舞いは owner が遅れたときだけ変わる（§3.8） |

## 1. なぜ，何を

X3x0 は chain ごとに状態を持ち越す：DDC/DUC の時刻つきの値，DUC が最後に見た header の時刻，閉じていない burst，止めたかどうか分からない streamer（設計メモ 39 §6.1，VERIFIED）．設計メモ 39 §6 はこれに**振る舞いの規則**（T1〜T3，R2′）で答えた．規則を実装すると，その状態を表す値がコードの中にまた散った．

- **受信**（D9b-2）：chain が流れているかを `RxDsp.stopped`（共有），`Stream.stopped`（owner だけ），`Phase::Active`（`Streams`）の三つが別々に表す．渡した DDC の値（obligation）は stream ごとの `RxDsp.pending` にあり，誰が始末するかは「owner のどの出口も始末を呼ぶ」という約束だけで決まる．
- **送信**（D9b-1a）：device burst の状態を `open`，`continues_at`，`tail`，`front`，`reopen_needed` の組み合わせが表す（`tx.rs:113-135`）．どの組が許されるかは書かれていない．

D9b-2 の 6 件はどれも「この組み合わせのときに，この出口で，あの値が古い／無い」の形だった（§2.3）．D9b-1b は送信に obligation と underflow の後の続き（owner の (a)）を足すので，同じ形の値がさらに増える．

**この見直しで決めること**：

1. **装置の DDC には，今流している segment の値しか置かない**（§3.1）．uhd-control は，受信の周波数を，それを覆う segment をその streamer が流し始めてから時刻つきで渡す．それまでは今と同じく自分の held の列に持ち続ける．これは D9 の R1（設計メモ 42：「その instant を走行中の segment が覆う chain にだけ渡す」）の元の趣旨で，今のコードの「まだ始まっていない後続でも計画が覆えば渡す」（`control.rs:789-790`）という例外をやめる．
2. **stream ごとの装置の事実を一つの値にし，変えるのは遷移の関数だけにする**．装置が流れているか，owner が何をしようとしているか（cut，すぐ止める，終わる），装置に渡した値．これらは同時に成り立つので，一つの enum にはまとめず，同じ値の中に並べる（Sol 1 回目 1・6）．

1 があるので，装置に残る値の持ち主はいつも，その segment を流している owner 一人になる．持ち越し，宛先の照らし，持ち主の移し，stream をまたぐ受け渡しは，どれも要らない．設計メモ 35 が `Streams` の印（holder と段階）にしたことを，装置の chain に広げる．

## 2. 今の形

### 2.1 受信の chain を表す値（main `22adbb7`）

| 値 | 置き場所 | 書く | 読む | 意味 |
|---|---|---|---|---|
| `RxDsp.pending`・`reported` | `core.rs:99-108`，stream ごとの `ClockMutex` | uhd-control の `give`（登録，失敗の取り消し，受理の時刻．`control.rs:854-891`），uhd-rx の `consume_rx_dsp`・始末 | uhd-rx（drain の期限，始末），uhd-control（取り消す entry を探す，`control.rs:875`） | 渡した時刻つきの DDC の値と，報告済みかどうか |
| `RxDsp.stopped` | 同じ | uhd-rx（`try_end`，`flush_rx`，`begin`，`refuse_segment`） | uhd-control（`give` の格下げ，`control.rs:850`），uhd-rx（restore の前） | 装置が止まっているか |
| `RxDsp.latest`・`restore`・`restore_ready` | 同じ | uhd-control（`give`，`finish`，`control.rs:384-415`） | uhd-rx（`try_end`） | 最後に受理された値（消費の後も残る），`Provider::stop` の S で戻す値，それが用意できたか |
| `Stream.stopped`・`cut` | uhd-rx だけ（`rx.rs:53-73`） | uhd-rx | uhd-rx（`recv_len`，`try_end`） | 装置に stop を出したか，どこで切るか |
| `Rx.stream`（`Some`/`None`）・`exit`・`stop_requested`・`last_stop` | uhd-rx だけ（`rx.rs:120-143`） | uhd-rx | uhd-rx | segment を処理中か，終わりの命令（`Provider::stop` か命令の channel の切断）を受けたか，cut を待たずに止めるか |
| `Phase::Active { segment, delivered }` | `Streams`（設計メモ 35 R2） | uhd-rx（`started`，`finished`） | uhd-control（`give` の `active_covers`），booking の End | owner が処理中の segment |

最初の segment の start は `Provider::start` が `Rx` を作る前に出す（`mod.rs:676-690`）．後片付けの close は `close_device` を通らず `device.close_streams()` で全部を閉じ，detach された thread があれば閉じない（`mod.rs:770-776`）．`owner_configure` は後続の準備で古い streamer を閉じてから `await_release` で待つ（`core.rs:1476-1495`）．

### 2.2 送信の device burst を表す値

`open`（Kernel burst を送っている．`first` は最初の送りがまだか），`continues_at`＋`tail`（device burst が続きを待って開いている，最後の 1 sample を持っている），`front`（次に安全に始められる sample），`reopen_needed`（EOB の送りが失敗し，streamer を開き直すまで送らない）．どれも uhd-tx だけが持つ（`tx.rs:96-137`）．これらは同時に成り立ちうる：新しい `open`（`first`）と前の device burst の `continues_at`・`tail`，送りの barrier で止まった `open` と `reopen_needed`，開いている burst と残っている `front`．開き直しの失敗は segment を止めて burst を始末する（`tx.rs:771-802`）．時刻つきの DUC の値は uhd-control が channel ごとの gate の下で渡す（`core.rs:822`，`control.rs:813`）．送信には obligation がまだ無い（D9b-1b で入る）．

### 2.3 D9b-2 の 6 件をこの表で読む

| 指摘 | 散った値 |
|---|---|
| 古い `stopped` が後続の時刻つきの値を時刻なしにする | `RxDsp.stopped` と `Phase::Active`（二つの lock） |
| flush の stop の失敗の後に restore | `RxDsp.stopped` を flush が書き直す |
| 完了と受け渡しの競合 | `Phase::Active`（`streams`）と `RxDsp`（chain の lock）が別の時点で変わる |
| 拒否された後続の値に持ち主がいない | obligation の持ち主が「出口が始末を呼ぶ」約束だけ |
| `await_release` の取り消し，移動元の拒否，stream の無い shutdown | 同じ |
| `pending_settled` | 「始末した」を状態でなく印で覚えた |

1 件目と後ろの 4 件は，まだ始まっていない後続に値を渡したことから出ている．§3.1 ではそもそも起きない．D9b-1a の 6 件のうち，送信の回復の印の組み合わせ（失敗した close の後の送り，普通の Stop が回復を消す，失敗した開き直し）は同じ形で，残りは FakeDevice の DUC・radio・header の model だった（`reviews/d9b1a-sol.md` の表）．後者はこのメモの範囲外（差分の harness が固める）．

## 3. 推奨：受信

### 3.1 規則：時刻つきの DDC の値は，その segment を流している streamer にだけ渡す

uhd-control の release（`control.rs:676-748`）は今，held の列を instant の順に見て，release window に入った値を渡し，渡す先の channel が無い値に当たると「残して，その回の pass をそこで止める」（後ろの値が先に装置へ行かないように．#61，#64）．受信の周波数について，渡す条件を一つ変える．

- **今**：計画で `e` を覆う segment（拒否されていない）があれば時刻つきで渡す（`reach`，`control.rs:780-802`）．その segment がまだ始まっていなくても渡す．
- **第 6 版**：`e` を覆う segment があり，**その stream の chain が今その segment を流している**（`Armed` か `Streaming`．§3.2）ときだけ時刻つきで渡す．覆う segment はあるが，まだ流していない（後続で，owner がまだ始めていない；古い segment が終わりかけ）なら，今の「channel が無い値」と同じく held に残し，その回の pass を止める．次の pass（`POLL` は 1 ms，`control.rs:37`）で見直す．
- 覆う segment が無くなった（拒否，取り消し，計画の変更）なら，今の `targets` の道（後の segment，configure の projection，`no channel`）にそのまま落ちる．uhd-control は毎 pass 計画を読み直すので，行き先の決め直しは今の仕組みのまま．
- 判断は chain の lock の中でする（今の `give` が `rx_dsp` を取るのと同じ所，`control.rs:831`）．owner は segment を始める（`rx_start`）とき，終える（stop・finish）ときに同じ lock を取るので，「流している」の判断と渡しは一つの節で決まる．

これで，装置に渡した値（debt）は**いつも今流している segment の値**になる．値を装置に渡すのは今と同じく uhd-control だけで，順番も今と同じく held の列の instant の順．stream をまたぐ受け渡しも，owner からの受け渡しも無い．

### 3.2 値の置き場所と lock

受信 stream ごとに一つ（今の `rx_dsp` の `ClockMutex` のまま．instance に一つの lock は stream どうしを待たせる．Sol 3 回目 1）．

```rust
pub(crate) struct RxChain {
    device: Device,                 // Off | Armed { at } | Streaming | Flushing | Uncertain | Lost
    intent: Intent,                 // { cut: Option<i64>, end_now: bool, terminate: Option<StopMode> }．三つは独立
    segment: Option<SegmentKey>,    // 今流している segment（start から finish まで）
    debt: Vec<Entry>,               // 装置に渡した値．いつも `segment` の値．消費されると消える
    left: Vec<Entry>,               // 報告したが消費されなかった値（KL-9：装置に残る）
    latest: BTreeMap<usize, Entry>, // channel ごとに最後に受理された値．消費の後も残る（restore の材料）
    restore: Restore,               // NotAsked | Planned(BTreeMap<channel, value>) | Done
}
struct SegmentKey { by: Option<u64>, origin: i64 }   // by が None は最初の segment
```

- `Intent`：`cut` は計画から読んだ cut，`end_now` は cut を待たずに止める（静かな stream，cut のある LateCommand，読める長さが 0，abort．今の `stop_requested`），`terminate` は `Provider::stop` か命令の channel の切断（今の `exit`）．`terminate` は `cut` を消さない（Sol 2 回目 1）．
- lock の順：projection の `updates` → chain → `streams`．装置の呼び出しは `streams` の外．`give` は今この順で取る（`control.rs:812,831,837`）．owner は chain を持ったまま configure（`updates` を取る）や `await_release` の待ちを呼ばない．start，stop，flush の読み，時刻つきの apply は chain を持ったまま呼んでよい（今の `begin`，`try_end`，`give` と同じ．`ClockMutex` は待つ参加者を clock の上で idle にする，`host.rs:517-536`）．一つの stream の chain を持つ間に別の stream の chain は取らない．

### 3.3 遷移と装置の呼び出しの結果

| 遷移 | 誰 | 前の `device` | 呼び出し | 成功 | 失敗（loss でない） | loss |
|---|---|---|---|---|---|---|
| `adopt_start(key, at)` | `Rx::new` | `Off` | なし（`Provider::start` が出した） | `Armed{at}`，`segment = key` | — | — |
| `begin(key, at)` | uhd-rx（configure の後） | `Off`・`Uncertain` | `rx_start(at)` | `Armed{at}`，`segment = key`，同じ節で `Streams::started` | 前のまま（`Uncertain` は `Uncertain` のまま．失敗した start は前の stream が止まった証にならない），segment は拒否（今の `refuse_segment`） | `Lost` |
| `restart(at)` | uhd-rx（cut の無い LateCommand） | `Armed`・`Streaming` | `rx_start(at)` | `Armed{at}` | 前のまま（今と同じく記録して続ける．1 s 黙れば UR-29 の loss） | `Lost` |
| （cut のある LateCommand） | uhd-rx | `Armed`・`Streaming` | なし | `intent.end_now`（今と同じく終える，`rx.rs:605-610`） | — | — |
| `first_sample()` | uhd-rx | `Armed` | なし | `Streaming` | — | — |
| `hand(entry)` | uhd-control | `Armed`・`Streaming` で `segment` が `e` を覆う | `apply(timed)`（前に debt に登録，失敗なら外す） | debt に残る | 外して報告（今と同じ：行なし，`COMMAND_REJECTED`，config of record に入らない） | `Lost` |
| `stop()` | uhd-rx（§3.5 の期限） | `Armed`・`Streaming` | `rx_stop` | `Off` | `Uncertain` | `Lost` |
| `flush()` | uhd-rx（stop の後に debt が残り，期限が残る） | `Off` | `rx_start_now`（→ `Flushing`），one_packet の読み，`rx_stop` | 最後の stop が成功：`Off` | start の失敗：`Off` のまま；読みの失敗：今と同じく数えて続ける；最後の stop の失敗：`Uncertain`（restore しない） | `Lost` |
| `finish()` | uhd-rx（stop・flush の後，または loss の後） | `Off`・`Uncertain`・`Lost` | なし（`Lost` でも装置は呼ばない）．残る debt を `left` に移して，まだ報告していないものだけ `rx_drain` に一度報告し，`segment = None` | 同じ節で `Streams` の `finished`・`complete`・`prune`（loss の後の論理の完了も今と同じ，`rx.rs:232-236`） | — | — |
| `lose()` | loss を見つけた thread | どれでも | なし | `Lost`（戻らない．`adopt_start` の時にもう loss が公開されていても `Lost`）．debt を一度だけ報告 | — | — |
| `abandon_unowned()` | cleanup（owner の thread を起こせなかった stream） | `Armed` | なし（`close_streams` が閉じる） | 状態を記録 | — | — |

- `Uncertain` からは次の segment の `begin` に進める（今と同じ．KL-9 の扱いを保つ）．
- 読みの one_packet：今の条件（`now ≥ cut`）のまま．flush の読みは常に one_packet（Sol 1 回目 5）．

### 3.4 debt の始末

debt は今流している segment の値だけなので，その持ち主は owner 一人．owner は segment を終える前に drain し（§3.5），残れば一度 flush し，それでも残れば `finish` で `left` に移して報告する．segment を終えない限り，owner は claim を返さず，streamer を閉じず，次の segment を始めない（今の `try_end` → `finish` → `begin` の順のまま）．`begin` は debt が空の chain でしか起きない．だから debt に「持ち主のいない」時は無く，宛先を照らす仕組みも要らない．

`left`（報告したが消費されなかった値）は KL-9 のまま：次にその channel を流す sample が消費する．

### 3.5 drain の期限

今と同じく「debt の最も遅い instant＋受信 1 回＋allowance（下限 cut＋quiet_span）」．debt は今の segment の値だけなので，後続の値で延びない（§5 が消える）．

### 3.6 streamer を閉じる・開く

streamer を閉じる・開き直す・開く全ての道（`owner_configure` の閉じ直し，`release_unneeded`，uhd-control の from-none の open と rate の確かめの失敗の後の close，`open_streamer` の失敗の後の close，prepare，cleanup の `close_streams`．Sol 3 回目 5）で，その stream の chain は segment を流しておらず，debt は空である（`left` は残りうる）．不変条件として確かめる：閉じる・開く呼び出しの前に，chain の `device` ∈ {`Off`, `Uncertain`, `Lost`} で debt が空（test build で違えば panic）．例外：owner の thread を起こせなかった stream（`abandon_unowned`）．cleanup は detach された thread があれば閉じない（今と同じ）．

### 3.7 restore の材料

`latest` は channel ごとに，消費の後も残す（今の `RxDsp.latest` と同じ役目．Sol 2 回目 6）．uhd-control の `finish` は今と同じく，S の後に効きうる値（`e > S`，または受理が S より後）のある channel を `latest` から選び，`Restore::Planned` にする．値を装置に渡すのは uhd-control だけで，`Provider::stop` の後は渡さない（`control.rs:838-840`）ので，restore の計画の後に debt は増えない．uhd-rx は `terminate` の後，その channel について，どの stream の chain にも debt と `left` が無く（chain の lock を一つずつ取って読む），自分の `device` が `Off` のときだけ restore する．

### 3.8 見える振る舞いの変わる所（owner が遅れたときだけ）

| 場合 | 今 | 第 6 版 |
|---|---|---|
| release の時（`e` − release window）に，覆う segment をその streamer がもう流している．普通の場合：後続の begin は古い segment の終わりの後すぐで，origin の lead 前（§5 の式で，release の始まりと古い drain の上限の差は `L − W` ≥ 0） | release の時に時刻つきで渡る | 同じ |
| release の時に，覆う segment をまだ流していない（owner が遅れた．`L = W` なら owner が lock を後に取るだけで起きうる） | release の時に渡る．古い segment が終わりかけなら，その drain がこの値まで延びる（§5） | held に残り，その pass は止まる．streamer がその segment を流し始めた次の pass で時刻つきで渡る．その間，held の列でこの値の後ろの値（他の channel と送信も）も待つ（今の「channel が無い値」と同じ扱い） |
| その結果，値が装置に着くのが `e` に近い，または過ぎる | — | DDC は `e` 以後の sample で取り出すので，着いた後の最初の該当 sample から当たる．radio の時刻つきの命令（LO）は，時刻を過ぎて着けばすぐ実行される（UHD の手引き「past-due commands execute immediately」）．timed tune の位相の同期は保証しない（OBX は `e` の約 400 µs 後の参照に合わせた GPIO の操作を予約する，`obx_expert.cpp:27-59`．時刻を過ぎて着いたときの物理の位相は INFERRED，実機で未確認） |
| 後続の start が遅れて LateCommand（cut が無い）で再開 | 値は前もって DDC で待ち，再開の後の `e` 以後の sample に当たる | 最初の start で chain はもう `Armed` なので，値は再開の前にも後にも渡りうる（どちらも同じ受け入れの規則）．`e` 以後の sample に当たる．`e` を過ぎて着けば上の行と同じ |
| cut のある LateCommand | owner は終わりを求める（再開しない） | 同じ．その segment の debt は drain と flush の規則で始末する |
| held の列の後ろの値（他の stream，送信） | release window で渡る | 待っている値より後ろの値は，その値が渡るまで待つ．他の stream の owner が時間どおりに始めていても遅れる（head-of-line）．owner が遅れた間だけ |
| 値を持っている間に `Provider::stop` か loss | release の時に渡っていれば `issued` | 今の `settle` の規則のまま（S と同じ instant の扱いも，`control.rs:907-938`）：渡していない値として，S まで（loss は S より前）に効くものは `no channel` か取り消し，S より後は取り消し |
| 値を持っている間に覆う segment が消える（拒否，取り消し，cold change） | release の時に渡っていた | 今の `targets` の道に落ちる：後の segment の configure の projection で当たる，configure 済みの後の segment なら release の時に時刻なしで渡る，どれも無ければ `no channel` |
| 覆う segment がいつまでも始まらない | — | その値と後ろの値は，`Provider::stop`，拒否，取り消しのどれかが来るまで待つ．その道で上の二つの行のとおりに始末される |
| 渡した値の apply の拒否 | 行なし，`COMMAND_REJECTED`，config of record に入らない | 同じ（渡す時点が後ろに動くだけ） |
| `applied` の行 | release の後，instant に `issued` | 値を `e` の後まで持っていれば，`issued` の行は渡したときに書く（行の `at` は `e`）．渡す前に S が来れば上の `settle` の行 |

値を持っていたこと自体は新しい event にしない（予約の時の `LATE_COMMAND` と start の再開の `LATE_COMMAND` は今のまま）．記録の `applied` の行の `at` は `e` のまま．Mock の論理の時刻と VH-4 の parity は変えない．物理の遅れは owner の遅れのときだけ．spec の改め（L1 と同じ commit）：UR-24 の「Holding and release」（受信の周波数は覆う segment を streamer が流すまで held に残り，その pass は止まる）と記録の段落（`issued` の行を書く時点），「A receive frequency and the DDC」（上の表の結果），「Consumption」（debt は今の segment の値だけ）．MR-34 の Ceiling を広げる：今は「その segment の owner が start を早く出すこと」を条件にしているが，uhd-control の release が遅れる場合（別の stream の owner の遅れによる head-of-line も）を含める．論理の分類と VH-4 の論理の origin は変えない．MA-1 の Ceiling に「時刻を過ぎて着いた timed tune は位相を保証しない」を足す．KNOWN_LIMITATIONS に一つ（owner が遅れると，後続の最初の周波数とその後ろの命令が遅れて当たる）．

### 3.9 変えないもの

- 計画（`timeline::Lines`），`Streams` の holder と段階（設計メモ 35），装置の記録（`set`・`generation`・`configured`），loss の公開（`device_gate`），`Streams::end`．
- MockRadio と語彙（MockRadio は装置の chain を持たない．RM-10）．
- 値を装置に渡すのが uhd-control だけであること，held の列の順番．
- KL-9 の扱い（失敗した flush の後も同じ chain を使う）．
- **#67** は解かない：遠い先の start を出した streamer に `Provider::stop` が来ると，uhd-rx は今と同じく origin を待つ（`rx.rs:208`）．#56 の実機を待つ．

## 4. 推奨：送信（`TxChain`）

Kernel burst の進み（`open`：どの burst のどこまで送ったか，`first`）は uhd-tx に残す．streamer ごとの値に，装置の側の事実を三つ並べる（Sol 1 回目 6）．

```rust
pub(crate) struct TxChain {
    burst: DeviceBurst,                      // Closed | Open { next: i64, tail: Option<Tail> }
    front: Option<(ClockDomainId, i64)>,     // 次に安全に始められる sample．burst と独立
    recovery: Recovery,                      // Clean | ReopenNeeded | Halted
}
```

- `DeviceBurst::Open { next, tail }`：device burst が開いていて，次の sample が `next`．今の `continues_at`＋`tail` と，`open` が送り始めた後の状態の両方を表す．新しい Kernel burst（`first`）が前の device burst を続けるかは，この値と uhd-tx の `open` の組で決まる．
- `Recovery`：`ReopenNeeded` の間は payload を送らない（今の barrier）．普通の Stop は `ReopenNeeded` を消せない．出口は，開き直しと T3 の成功（`Clean`），開き直しの失敗（`Halted`：segment を止めて burst を始末する，今の `halt_failed_reopen`），`Provider::stop`（回復をやめる），loss．`Halted` は次の segment の switch で `Clean` に戻る．
- D9b-1b はここに channel ごとの debt（時刻つきの DUC の値）を足す．§3.1 と同じ考えで，DUC の値を渡す条件を D9b-1b で決める（DUC は burst の間の値を次の burst の最初で取るので，受信とは消費の条件が違う：設計メモ 39 §6.1 F4）．送信の消費の証拠は「装置が受け取った sample」で，送りの submit や物理の出力とは分ける．`Open` から `Closed` へ閉じる遷移は，debt の最も遅い instant の後まで 0 で延ばしてから EOB を出す（送信の R2′）．underflow の後の (a) は `Open → Closed → Open`（新しい時刻）．

## 5. この見直しで見つかった不具合（Sol が source の上で確かめた）

drain の期限は，その stream の debt **すべて**の最も遅い instant から測っている（`rx.rs:835-841`，`try_end` の `rx_drain_deadline(..., &dsp.pending)`）．

1. 古い segment が `c` で切られ，owner はまだ終えていない．
2. 後続の origin を `O`，release window を `W`，受信 1 回を `C`，allowance を `A`，lead を `L` とする．`O = c + W + C + A + L`（設計メモ 39 §6.7）．uhd-control は `e = O` の周波数を `now ≥ O − W` で release する．
3. uhd-control が古い owner の最後の判断より先に chain の lock を取る．後続が `e` を覆うので `reach` は `Timed`，古い chain の `stopped` はまだ false なので格下げされない（`control.rs:789-790,850-858`）．
4. entry は stream の debt に入る．
5. `try_end` は `e + C` ごろまで待つ（`rx_drain_deadline` がその entry を数える）．後続の timed start は origin までに出せない（LateCommand による再開と gap）．

release の始まりは `c + C + A + L`，古い drain の期限の上限は `c + W + C + A`．差は `L − W`．lead が最小（`L = W`，設計メモ 43 の規則 2 の下限）なら差は 0 で，uhd-control が lock を先に取れば起きる．第 6 版では，後続の値は後続が流れ始めるまで held に残る（§3.1）ので，古い segment の debt に入らず，起きない．

## 6. テストで固める系列（risky な段階の系列の表の元）

RX：
1. §3.3 の表の各行（前の状態，呼び出し，成功・失敗・loss の後）を，thread を使わない unit test で．`Uncertain` からの begin，`Streaming` 中の cut の無い LateCommand の再開，cut のある LateCommand の終わり，最初の segment の取り込み，flush の start の失敗と最後の stop の失敗，owner の thread を起こせなかった stream の cleanup．
2. §3.1：segment を流している間の値は release の時に渡る．覆う segment をまだ流していない値は held に残り，後ろの値も待ち，streamer が流し始めた次の pass で渡る．release と `begin` が同時に来る並び（gate つき）で，値が装置に一度だけ渡り，held の列の順番が保たれる．
3. §5：lead が最小（`L = W`）で，古い segment の終わりかけに後続の `e` の値が来ても，古い drain は延びず，後続は origin に始まり，値は `e` に当たる．
4. §3.8 の遅れの場合：owner の begin が release window の中に入ったとき，`e` を過ぎたとき，cut の無い LateCommand の再開の前と後，cut のある LateCommand．`applied` の行の `at` は `e`，値を持っていたことで新しい event は出ない．
4a. loss：受け渡しの最中，普通の受信の最中，flush の最中の loss で，`Lost` から `finish` が装置を呼ばずに segment を終え，debt を一度だけ報告する．
5. 覆う segment が消えた値（拒否，取り消し，cold change が `e` の前で切る，move の行き先の変更）は今の `targets` の道に落ちる（projection，時刻なしの release，`no channel`）．`Provider::stop` と loss の間に持っていた値は今の `settle` の規則（S と同じ instant を含む）．覆う segment がいつまでも始まらないとき，stop・拒否・取り消しで待ちが解ける．どれも装置の呼び出し，行の有無と結果，その後の configure の値を確かめる（行の `at` だけでなく）．head-of-line：別の stream の値が待つ間，時間どおりの stream の値も待つ．
6. debt の始末：segment を終える全ての道（cut，abort，`Provider::stop`，cold change，loss，LateCommand，静かな stream）で，debt は消費か報告（entry ごとに一度）になってから，claim を返し，streamer を閉じ，次の segment を始める．
7. 閉じる・開く全ての道（§3.6）で，chain の debt が空で `device` が `Off`・`Uncertain`・`Lost`（不変条件の panic を test build で確かめる）．
8. restore：S の前に消費され，`e` か受理が S より後の値の channel は restore される（`latest` が残る）．`left` のある channel は restore しない（move の後に別の stream の chain にある `left` も）．
9. 遠い cut の前の読みは one_packet にならない．D9b-2 の既存のテスト（`ur_24_*`，`ur_26_*`）と差分の harness（P4 A/C）は通る．後続に前もって渡した値を確かめていたテストは，渡る時点を後続の begin の後に直す（どれを直したかを報告に一つずつ）．

TX：
10. `DeviceBurst`・`front`・`Recovery` の組と uhd-tx の `open` の組の表．新しい `open`（`first`）が前の device burst を続ける場合，barrier で止まった `open`，開いている burst と残る `front`．
11. `Recovery` の出口：開き直しの成功，失敗（`Halted`）と次の segment での `Clean`，普通の Stop（消えない），`Provider::stop`，loss．
12. D9b-1a の既存のテスト（T1〜T3，`tx_close`，開き直し，P5〜P7・D70 の harness）がそのまま通る．

## 7. 採らない案

- **範囲を絞る（今の受け渡しを保ち，値に宛先を持たせて drain の期限だけ直す）**：見える振る舞いは一つも変わらないが，「まだ始まっていない後続に渡す」例外が残る．そのため宛先を照らす仕組みと，owner がひどく遅れたときの既知の制限が恒久的に残る．owner 2026-10-30：最終形のきれいな方を採る．
- **始まっていない後続にも uhd-control が時刻つきの値を渡し，古い chain の drain から外して持ち越す**（第 1〜3 版）：持ち越してよい条件，後続の configure が遅れたとき，消費された値と projection の合わせ方，move で誰が始末するかが，それぞれ規則を要した．Sol の 3 回のレビューは毎回新しい穴を見つけた．
- **まだ渡していない値を chain の `waiting` に置き，owner が segment を始めるときに渡す**（第 4・5 版）：stream をまたいで `waiting` を移すとき lock の持ち替えに隙間ができ，その間に移し先が始まる，または uhd-control の新しい値が先に渡って装置の queue の順が逆になる（Sol 5 回目 1）．owner が装置に値を渡す二人目の書き手になる．第 6 版は uhd-control が持ち続けるので，書き手は一人，順番は held の列のまま．
- **debt を channel ごとに instance の一つの lock で持つ**（第 3 版）：一つの stream の flush の間，他の stream の受信と受け渡しが待つ（Sol 3 回目 1）．§3.1 の後は装置に値を持つのはその channel を流している stream だけなので，stream ごとの lock で足りる．
- **stream ごとに装置を一つの thread（actor）に持たせ，uhd-control は message で頼む**：受け渡しは `e` の release window 前に装置に着かねばならないが，uhd-rx は受信の呼び出しで最大で block の長さ（1 MS/s で 20 ms）待つ．message はその後ろに並ぶ．範囲も大きい（設計メモ 35 §5 と同じ理由．Sol も同意）．
- **今の値を残し，出口ごとの debug assert を足す**：新しい出口が assert を通らなければ見えない．D9b-2 の 3 件はまさに新しい出口だった．
- **chain の状態や debt を `Streams` や timeline に入れる**：`streams` の lock は装置の呼び出しをまたいで持てない（`core.rs:129-130`）．debt は装置に残った物理的な状態で，計画の意図ではない（Sol 同意）．
- **一つの enum にまとめる**（第 1 版）：同時に成り立つ事実を表せない（Sol 1 回目 1・6）．
- **送信は D9b-1b の中で直す**：D9b-1b は debt，R2′，(a)，#71 を足す大きな段階で，形の変更を混ぜると review が難しくなる．動作を変えない段階を先にする（Sol 同意）．

## 8. 大きさと分け方

- **L1（受信，risky）**：§3.1 の規則（uhd-control の release と `reach` の変更），`RxChain`（§3.2 の値，§3.3 の遷移，§3.4 の始末，§3.6 の不変条件），UR-24・MA-1 の改め，KNOWN_LIMITATIONS．D9b-2 の宛先のない値の始末（出口ごとの settle，`refuse_segment` の recovery，`owner_configure_with_abort` の callback）は §3.1 で要らなくなるので消す．約 1,500 行（本体とテスト）．
- **L2（送信，risky）**：`TxChain`（burst，front，回復）．動作を変えない．約 1,000 行．
- その後に D9b-1b（送信の debt，R2′，(a)，#71）．

各段階は AGENTS.md §9 の risky の手順（系列の表 → テスト計画 → Sol の計画レビュー → 実装）．依る挙動は Module の中のものと，設計メモ 39 §6.1・P4 C の VERIFIED の事実，§3.8 の UHD の手引きの事実（past-due の命令はすぐ実行）．物理の位相（§3.8）は INFERRED．

## 9. owner に決めてもらうこと（一つずつ）

1. **範囲**：受信（L1）と送信（L2）の両方を D9b-1b の前にする（推奨．Sol も同じ）．代わり：受信だけ先にし，送信は D9b-1b の中で．→ **owner 2026-10-30：両方**．
2. **§3.8 の振る舞いの変化**：owner が遅れたときだけ，後続の最初の周波数が遅れて当たり（位相は保証しない），held の列の後ろの値も待つ．これを UR-24・MA-1 の Ceiling と KNOWN_LIMITATIONS に書いて受け入れる（推奨）．代わり：遅れの場合だけ今の「前もって渡す」を残す（例外が戻り，§2.3 の形が残る）．→ **owner 2026-10-30：受け入れて spec に書く**．
3. **実機の時期**：L1 と L2 の後に一度（推奨．今の予定では実機の日がまだ先）．代わり：L1 の後に一度，L2 の後にもう一度．→ **owner 2026-10-30：L1 と L2 の後に一度．その前に実機の日が来たら，その時点の main で確かめる**．

## 10. レビュー

Sol 6.1，1 回目（2026-10-30）：REVISE．方向は正しいとし，major 6・minor 1．(1) 受信の表に無い道 → 事実を分け，遷移を足した．(2)「Idle の証」は閉じる理由と合わない．(3) `hand` を configure 済みに狭めていた，宛先が消えたときの規則が無い．(4) §5 は実在するが時刻の条件と直しを正確に → §5 を Sol の系列で書いた．(5) cut を知っただけで one_packet にしない → 今の条件のまま．(6) 送信の enum は同時に成り立つ組を表せない → burst・front・回復に分けた．minor 7（事実の誤り，lock の順）→ §2，§3.2．

Sol 6.1，2 回目（同日）：REVISE．major 6．(1) `Terminate` が cut を消す，失敗の結果が表に無い，LateCommand を最初の sample の前に限っていた → `Intent` を三つの独立した値に，呼び出しごとに成功・失敗・loss の結果を書いた．(2)「毎 turn」では閉じる前と `await_release` の中で間に合わない．(3) DDC は channel ごとで，move で stream を移る．(4) 持ち越してよい条件と，UR-24 の改めが要る．(5) 許可の条件と lock の範囲が無い．(6) `latest` を落としていた → §3.7．

Sol 6.1，3 回目（同日）：REVISE．major 5：(1) instance に一つの lock は stream どうしを待たせる → stream ごとの lock に戻した．(2)〜(4) 持ち越しから出る穴（configure の時刻，消費された値と projection，move の後の持ち主）．(5) 閉じる・開く道の抜け → §3.6．

Sol 6.1，4 回目（同日）：REVISE．持ち越しをやめたことと stream ごとの chain は支持．major 5：(1) 時刻なしの代わりは同じではない（位相の同期を失う）→ いつも時刻つき．(2) `waiting` の行き先は覆うかどうかで決め直す．(3) start の時の apply の拒否で行が変わる．(4) owner の受け渡しが S の後に debt を作りうる．(5) owner の thread を起こせなかったとき cleanup が不変条件を破る → §3.3 の `abandon_unowned`，§3.6．

Sol 6.1，5 回目（同日）：REVISE．major 2・minor 1：(1) stream をまたいで `waiting` を移すときの順番の約束が無い．(2) 遅れて着いたときの表が言い過ぎ（最大の遅れは device lead ではない，位相の同等は示せない，cut のある LateCommand は再開しない）．(3) §3.8 の「変えない」と食い違う．

owner の決め（2026-10-30）：範囲を絞る案（今の受け渡しを保つ）と比べ，最終形のきれいな方を採る．第 6 版は `waiting` と owner からの受け渡しをやめ，uhd-control が値を持ち続ける形にした（§3.1）．これで 4・5 回目の受け渡しの指摘（`waiting` の移し，順番，S の後の owner の受け渡し，start の時の拒否の行）は起きない．5 回目の (2) は §3.8 の表を書き直した（遅れの上限を言わない，位相は保証しない・INFERRED，cut のある LateCommand は終わり）．(3) は §3.8 と §3.9 を揃えた．

Sol 6.1，6 回目（同日）：**ADOPT WITH CHANGES**．中心の規則（uhd-control が持ち続け，held の順を保ち，chain の lock の下で今の segment が覆うときだけ debt を作る）は健全，`waiting` や owner の受け渡しに戻らないこと．D9b-2 の宛先のない値の始末を消すのも妥当．変更：(1) §3.8 の表と spec の改めの範囲が足りない（head-of-line，持っている間の stop・loss，覆う segment が消えたときの道，`issued` の行の時点，UR-24 の Holding and release と記録の段落，MR-34 の Ceiling）→ §3.8 と §6 の 5 に入れた．(2) loss の後の `finish` の道が無い → §3.3 で `Lost` からの `finish` を足し，§6 の 4a．(3) LateCommand の言い方（渡すのは再開の前でも後でもありうる，予約の時の `LATE_COMMAND` は残る，cut のある LateCommand の行）→ §3.8．note：始まらない segment が後ろの release を止め続けうることを書き，stop・拒否・取り消しで解けることを試験で確かめる → §3.8，§6 の 5．L1 のテスト計画に進んでよい（所有の設計のやり直しは要らない）．
