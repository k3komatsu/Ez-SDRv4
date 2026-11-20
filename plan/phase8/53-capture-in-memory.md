# 53. ファイルを経ない capture（#73）と大きな artifact の `read`（#97）— 設計メモ（決定版）

## 0. 状態

| 項目 | 内容 |
|---|---|
| 状態 | **決定**（2026-11-20）．AGENTS §9 step 0 を通った：Opus の下書き（`~/.cache/ezsdr-fixes/2026-11-20-n73/draft.md`）→ owner の判断（下の行）→ Fable 5.2 の設計レビュー（[reviews/n73-fable.md](reviews/n73-fable.md)．owner の判断と orchestrator の決定の記録は [reviews/n73-decisions.md](reviews/n73-decisions.md)）→ owner の判断 R1〜R4 → orchestrator の処置（§7）．§8 の O1・O2 も owner が推奨のとおり決めた（2026-11-20）．owner に残る点は無い |
| owner の判断 | **方向**（2026-11-20「はい、これでいいです」：memory の既定は Python client に置き，wire の `sink.capture` は file のまま，in-memory の capture に Vocabulary の verb を一つ足す，server が Session ごとに bytes を持ち一度だけ読ませる，Manifest の記録は file と同じで URI が `mem:`，Kernel は変えない）．**D1**（2026-11-20「ok, まだユーザーは私だけです」），**D2**（同日「代案かな」→ verb `sink.capture_in_memory`），**D3**・**D4**（同日「ok」），**D5**（同日「新しいのを捨てるでok」→ 同日 **改め**「はい、これでよいです」：古い順に追い出す），**D6**（D5 改めと同じ返事），**D7**（同日「ok」），**D8**（同日「ok」），**D9**（同日「はい」），**D10**（同日「ok」），**R1**〜**R4**（同日「ok」．R2 は owner 自身の案）．下書きと違う所はすべてこれらが勝つ |
| issue | #73「Capture without a file」（enhancement，area:capture-provenance），#97「EA-13 read loads a whole artifact into the server's memory (and the client's)」（bug，area:capture-provenance，phase:8b）．#73 は S4 で，#97 は S1 で閉じる |
| 根拠の commit | `work/bt2` HEAD `500726dd`．`crates/ezsdr-server`・`crates/ezsdr-sink-capture`・`crates/ezsdr-sink`・`crates/ezsdr-acceptance/src/rig.rs`・`crates/ezsdr-kernel/src/{stream,spec.rs,session.rs}`・`python/`・`design/10-host-data-path.md`・`design/16-easy-api.md`・`KNOWN_LIMITATIONS.md`・`design/02`・`design/vision/`・`plan/phase7/00-overview.md` は origin/main `62d947d0` と同じ（`git diff --stat HEAD origin/main -- <path>` が空．VERIFIED）．違うのは Kernel の `coordinator/{admission,ending,mod,paced,pipeline,record,state,stepping}.rs`・`event.rs`・`manifest.rs`，`design/04`・`05`・`06`・`07`・`09`・`18`，`plan/phase8/50-remaining-work.md`，`plan/maintenance/failure-mechanisms.md`（同じ command の全件）なので，それらの行は **origin/main `62d947d0`**（`git show origin/main:<path>` で読んだ）．下書きと Fable が引いた bt2 の Kernel の行（`stepping.rs:428-430`，`:204/214`）と下書きの `4679abc1` の行は使わない |
| 印 | **VERIFIED**：読んだ path:line で確かめた．**INFERRED**：推定（理由を書く） |

## 1. 結論

- **wire の `sink.capture`（key `sink.capture_samples`）は今の HD-10 のまま file**．Spec の schedule，Spec の output の own capture，raw の `submit` は変わらない．memory に取る capture は Vocabulary `sink` に足す verb **`sink.capture_in_memory`**（key **`sink.capture_in_memory_samples`**）で頼む（D2）．
- **Python の `capture()`/`request()` は既定で memory**（`keep=False` → `sink.capture_in_memory`），`keep=True` で今と同じ file（`sink.capture`）（D1）．
- **bytes は server が Session ごとに作る一つの store に置く**．server が Session の Sink を組み立てるとき builder で渡す．child Run と acceptance rig の Sink には store が無く，in-memory の request は HD-14 の `sink.REQUEST_REJECTED` で断る（D9）．
- **Manifest の記録は file のときと同じ `ArtifactRef`**（hash・`size_bytes`・`partial`・`continuity`），`uri` が `mem:<output id>/<artifact id>`．読まれたか・持てたかは書かない．schema は変えない（D3）．
- **上限は Session の bytes**（既定 1 GiB，`connect(capture_memory=...)`，`connected` が返す，BindingProfile には入れない）（D4）．数えるのは**実際に書いた bytes**で，Session の recorder 全部の和（R1）．
- **上限に当たったら，読まれていない完了済みの capture を古い順に追い出す**（取っている最中のものは追い出さない）．追い出しは memory が実際に要るときだけ（R1）．全部追い出しても入らない capture は持たない（記録はそのまま，専用の message）（R1）．一つで上限を超える capture は client が送る前に `Error`（D5 改め）．
- **`mem:` の `read` は一度きり**．二度目，追い出された後，持たなかったものは error kind **`not_held`**．捨てる op は足さない．`CaptureTimeout` に `.request` は付けない（D6）．
- **`result()` は読む前に continuity を見る**．gap・restart・channel 数の変化・invalid な区間・`partial` は `ezsdr.CaptureGap`（`Error` の subclass，`.artifact` つき）で，bytes は読まずに残す（D7，R4）．**`sdr.pieces(artifact)`** が一度読んで `Piece(start, samples, invalid)` の list に分ける（R2）．
- **#97：`read` は server の中で 64 KiB の buffer で流す**．frame は変えない．client は読む前に一度だけ確保して `readinto`，1 channel は copy しない．`read` に大きさの上限も range read も付けない（D8）．
- **server の `count()` が毎回全 event を clone する件は S3 で直す**（R3）．Kernel は変えない．
- **段取り**：S1（#97，risky でない，単独で先に）→ S2（Sink と store，risky）→ S3（server，risky）→ S4（Python と文書，risky でない）．S2 と S3 は一つの risky step の二つの小段にする（§6）．

## 2. 利用者から見た約束

### 2.1 できること

| 呼び出し | 変更後 |
|---|---|
| `connect(..., capture_memory=None)` | Session が memory に持つ capture の bytes の上限（int，bytes）．`None` は server の既定 1 GiB（1 073 741 824）．`sdr.capture_memory` が実際の値（`connected.capture_memory_bytes`）を返す（O2）．server の memory は読みの途中で最大でおよそこの 2 倍（§3 (c)） |
| `rx.capture(n, at=None, timeout=None, *, keep=False)` | 既定：`sink.capture_in_memory` を送り，samples を `numpy` で返す．Session directory に file は増えない．Manifest には `mem:` の `ArtifactRef` が残る．`keep=True`：今と同じ（`sink.capture`，file を書いて読む） |
| `rx.request(n, at=None, *, keep=False)` / `rx.result(h, timeout=None)` | 同じく．先に重ねた request は memory と file を混ぜても一つの queue で続けて取る（HD-10．連続した samples） |
| `rx.result(h)` で gap・restart・channel 数の変化・invalid な区間・`partial` | 読む前に `ezsdr.CaptureGap` を上げる．`e.artifact` が `ArtifactRef`．bytes は server（または file）に残っている |
| `sdr.pieces(artifact)` | `list[ezsdr.Piece]`．`Piece(start, samples, invalid)`：`start` はその piece の最初に届いた sample の `TimePoint`（その SampleClock の domain），`samples` は 1 channel なら `(n,)`，他は `(channels, n)` の `complex64`（書き換え可），`invalid` は channel ごとの `[(begin, end), ...]`（piece の中の sample 番号，end を含まない，有効なら空）．stream の gap，SampleClock の変化，channel 数の変化で分け，空の piece は返さない．channel ごとの invalid な区間では分けず，値は変えない．`mem:` は一度だけ読める．file は何度でも |
| `Session.read(artifact) -> bytes` | `file://` は今のとおり何度でも（server は流して送る）．`mem:` は一度だけ |
| `RunResult.capture(output)` | 変わらない（child Run の output は file）．`partial` を断るかは O1 |
| Spec の `outputs` と schedule の `sink.capture` | 変わらない（file） |
| raw の `sdr.submit(...)` で `sink.capture_in_memory` | 送れる．client の事前検査は無いが，server が上限で抑える（持たなければ `not_held`） |

### 2.2 できないこと・断られ方

error kind（EA-5）は今の `protocol`・`unsupported_protocol`・`refused`・`ended`・`not_on_primary_root`・`not_found`・`io` に **`not_held`** を一つ足す．Python では `ServerError(kind="not_held", message)`．

| 場面 | 見え方（message の形） |
|---|---|
| 一つの capture が上限より大きい（`n × channels × 8 > capture_memory`，`keep=False`） | `request()`/`capture()` が送る前に `ezsdr.Error`：`EA-17: a capture of {n} samples × {c} channels is {B} bytes, more than this Session's capture_memory of {C} bytes; pass keep=True to write it to a file, or connect(capture_memory=...)`．何も送らず，log に何も入らない |
| 読む前に追い出された（D5 改め） | `result()`/`pieces()`/`read()` が `ServerError` kind `not_held`：`EA-13: {uri} is no longer held: it was evicted, oldest first, to make room for later captures within this Session's capture_memory of {C} bytes; read results sooner, request with keep=True (sink.capture), or connect(capture_memory=...)` |
| 取っている最中の capture の和が上限を超え，追い出せるものが無かった，または一つで上限を超える raw の request（R1） | `not_held`：`EA-13: {uri} was not held: the captures in progress needed {B} bytes, more than this Session's capture_memory of {C} bytes; request with keep=True (sink.capture), or connect(capture_memory=...)`（「read sooner」は言わない） |
| server が確保に失敗した（`try_reserve`，R1） | `not_held`：`EA-13: {uri} was not held: the server could not allocate {B} bytes; request with keep=True (sink.capture)` |
| 一度読んだ `mem:` をもう一度（`result(h)` の二度目，`pieces` の二度目，`read` の二度目）（D6） | `not_held`：`EA-13: {uri} was read once and is no longer held; keep=True writes a file that can be read again` |
| 報告されていない URI（推測した `mem:` も） | 今のとおり `not_found`：`EA-13: {uri} is no artifact of this server's Runs` |
| `result()` の capture に gap など（D7，R4） | `ezsdr.CaptureGap`（`Error` の subclass）：`EA-17: {artifact id} {has a gap or an invalid stretch | spans {k} SampleClocks | does not have one channel count | is partial (cut short by a Stop or the Run's end)}: one array would hide it; sdr.pieces(e.artifact) returns its pieces (once, for a capture in memory)`．`read` は送っていない |
| `pieces()` で continuity と `size_bytes` が合わない | 送る前に `Error`：`EA-17: {artifact id}: its continuity accounts for {X} bytes of cf32, its size is {Y}; read its bytes with Session.read` |
| client が受ける array を確保できない（m4） | `read` を送る前に Python の `MemoryError`．Session は続き，`mem:` の bytes は残る |
| server の返す大きさが `size_bytes` と違う（file を外から変えた） | body を全部読んでから `Error`：`EA-13: read returned {X} bytes for {uri}, its artifact says {Y}`．Session は続く |
| 流している途中で server が body を書けない（m5） | server は EA-15 の道で Manifest を書いて終わる．client は今の `ProtocolError`（「the server's output ended inside a body」）で，Session は終わり |
| `connect(capture_memory=...)` が正の int でない（`0`，負，`bool`，`str`） | server を起こす前に `Error`：`EA-16: capture_memory must be a positive int of bytes, not {value!r}`（O2） |
| child Run（Spec）の schedule の `sink.capture_in_memory`（D9） | child の Sink が `sink.REQUEST_REJECTED`，reason `HD-14: this Run holds no capture in memory; use sink.capture`．child は続く |
| Spec の output の `params` に `sink.capture_in_memory_samples` | 今の HD-9 のとおり `prepare` が `Rejected`（`HD-9: params may contain only sink.capture_samples`） |
| in-memory の request の値が正の `Int` でない | `sink.REQUEST_REJECTED`，reason `HD-14: sink.capture_in_memory_samples must be a positive Int` |
| `capture(n, timeout=...)` が `CaptureTimeout` | 今と同じ．`.request` は無い（D6）．その request は後で取られ（EA-17），上限の中で追い出されるまで持たれる．取り戻したいなら最初から `request()`/`result()` |
| Session の終わりに読まれていない `mem:` | bytes は server と一緒に消える．Manifest には `mem:` の記録が残る |
| 長い monitoring | Session directory は増えない．action log・event・Manifest の `artifacts` は capture ごとに少しずつ伸びる（1 capture あたり約 1〜2 KB，INFERRED）．request ごとの遅れは S3 で除く（R3） |

## 3. 決まった設計（論点ごと）

### (e) 何で Sink に「memory に」を伝えるか（D2）

`sink.capture` は単一 key の `UpdateParameter` に compile され，他の params は compile で落ちる（`crates/ezsdr-kernel/src/session.rs:367-384`．VERIFIED）．Sink は key で振り分ける（`crates/ezsdr-sink-capture/src/lib.rs:181`．VERIFIED）．

- **決定：verb `sink.capture_in_memory` → key `sink.capture_in_memory_samples`（`int`，coercible でない，`block_boundary`）を HD-6 に足す**．Sink は二つの key を同じ queue に入れ，capture ごとに行き先（file か memory）を持つ．Kernel は変わらない．Vocabulary `sink` 1.1.0，Module `ezsdr.sink.capture` 1.2.0 の version は上げない（AGENTS §6）．
- 退け：`sink.take`（下書き）．owner が説明的な名前を選んだ（D2）．
- 退け：`sink.capture` の params に `sink.keep`．compile で落ちるので Kernel の `CompileRule` を複数 key にする必要（§3 の Kernel 成長）．
- 退け：`sink.capture` を `Command {}` に compile し直す．HD-6 の update class と RS-14 の timed update を作り直し，Spec の schedule にも響く．
- 退け：binding の selector（`memory: true`）．呼び出しごとに選べず，既定 profile の recorder は一つ（`design/16-easy-api.md:55-57`）．
- 退け：wire の `sink.capture` の既定を memory に．Sink は Spec の output と Session の request を見分けられない（HD-10，`design/10-host-data-path.md:45`）ので Spec の capture も file を失う．

### (a) protocol（D6，D8）

- **決定：op を足さない．`read { uri }` が `mem:` も読む**．`result()` は今も `wait_for` の後に `read`（`python/ezsdr/session.py:434-458`．VERIFIED）．`sink.CAPTURE_WRITTEN` の `artifact.uri` が `mem:` なら同じ道で bytes が来る．`connect` に `capture_memory_bytes?`，`connected` に `capture_memory_bytes`，`ErrorKind` に `not_held` を足す．frame の schema は再生成し，version は上げない（EA-6，AGENTS §6）．
- 退け：`result { action }` op（待ちと読みを一往復で）．`wait_for` の期限の扱い（EA-12，Review H P2-5）を server にもう一つ持つことになる．得は一往復だけ．
- 退け：`wait_for` の返事の body に samples．`wait_for` が Sink の event の意味を知ることになり，`events {}` で読む client に届かない．
- 退け：chunk ごとの frame．`mem:` は上限で抑えられ，file は (d) で流す．
- 退け：捨てる op（`remove_result` の形）．古い順の追い出しで要らない（D6，Fable §3）．

### (b) Manifest の記録（D3）

Kernel は Sink が返す `ArtifactRef` の URI を検べない：Sink の `stop()` の artifact は `keep_artifacts` がそのまま output の下に足す（origin/main `crates/ezsdr-kernel/src/coordinator/ending.rs:454-460`．VERIFIED）．input の locator としての `mem:` は既にある（origin/main `crates/ezsdr-kernel/src/coordinator/pipeline.rs:389,973`．VERIFIED）．

- **決定：file のときと同じ `ArtifactRef` を `artifacts` に，`uri: "mem:<output id>/<artifact id>"`**．hash と `size_bytes` は capture が記録した bytes 全部の上に積む（持てても持てなくても同じ値）．SigMF の meta は無い．Vision §50（`design/vision/09-provenance-validation-and-ownership.md:14`）と RS-44（origin/main `design/04-run-and-session.md:308`）をそのまま満たす．
- `<artifact id>` は `capture_<ActionId>`．ActionId は Run ごとの一つの atomic counter（origin/main `crates/ezsdr-kernel/src/coordinator/admission.rs:243`，`pipeline.rs:1226`．VERIFIED）で Run の中で一意．`Ident` は `[a-z][a-z0-9_]*`（`crates/ezsdr-kernel/src/spec.rs:46-49`．VERIFIED）なので input の `mem:sha256:<hex>` とぶつからない（Fable n3）．二つの Session の Manifest が同じ `mem:rec/capture_5` を持つことはある（bytes は Run の外に残らないので害は無い）．
- 退け：`ArtifactRef` に `kept: bool`（または `uri: null`）．Kernel の schema 変更．scheme で足りる．
- 退け：`artifacts` に載せず event だけ．`stop` が記録した最後の partial な capture の event は cleanup の drain の範囲でしか Manifest に届かない（HD-16 の末尾，`design/10-host-data-path.md:67`）．
- 退け：集計だけ．capture ごとの continuity を失い，log は action ごとに残るので伸びは止まらない．
- 退け：「持てたか・読まれたか」を書く．P1（下）に反する．
- 退け：`mem:sha256:<hex>`（content address）．同じ bytes の capture（simulation の無音）が store の key でぶつかる（Fable F7）．

**P1（決定の根拠）**：`read` は SessionAction ではなく log に入らない（`crates/ezsdr-kernel/src/session.rs:310-333`．VERIFIED）．だから Run の記録（event・`ArtifactRef`・Manifest）が「client がいつ読んだか」に依ると，log の再生（RS-20，origin/main `design/04-run-and-session.md:183`）と seed からの再実行（invariant 27）が分かれる．上限・追い出し・持たないことは store の中だけで効かせ，Sink の出すものを変えない．INFERRED：RS-20 の再生は今は profile hash の照合だけ（`crates/ezsdr-kernel/src/session.rs:160-162`）なので，P1 は今の挙動でなく約束を守るための原則．

### (c) store・上限・追い出し（D4，D5 改め，D6，R1）

**どこに持つか**

- **決定：store は `ezsdr-sink-capture` の型**（名前は実装に任せる．下では「store」）．server が Session の `connect` で上限つきで一つ作り，`catalogue::assemble`（`crates/ezsdr-server/src/catalogue.rs:90,115`．VERIFIED）に引数で渡し，そこが `CaptureSink` に builder で付ける．`run_child`（`crates/ezsdr-server/src/lib.rs:330`）と acceptance rig（`crates/ezsdr-acceptance/src/rig.rs:200,243`）は渡さない．Kernel は知らない．host が Module に資源を渡す形は `Config.open_device`（EA-7）と同じ．
- 退け：Sink が自分で持ち server が Kernel 経由で取り出す．Kernel に口が要る．
- 退け：tmpfs に書いて読んだら消す．owner が退けた `remove_result` の形で，Manifest が存在しない `file://` を指し，tmpfs の埋まりを上限で抑えられない．
- 退け：child Run にも store（D9）．child の結果は `RunResult.capture` が file を読む形で使い道が薄く，child の Manifest に読めない `mem:` が並ぶ．

**数え方（R1）**

- 上限 C に対して数えるのは，store にある完了済みの bytes（`Held`）＋ Session の全 recorder で取っている最中の capture が**実際に書いた** bytes．予約（capacity）は数えない．
- Sink は capture の始まり（`start_front`，`crates/ezsdr-sink-capture/src/lib.rs:299`）で，最初の block の frame（bytes per sample × channels）から `n × frame` を見積もる．
  - `n × frame > C` なら，その capture は**始めから持たない**（何も確保せず，何も追い出さない）．hash と continuity だけを積む．raw の `submit` だけがここに来る（Python は送る前に断る）．orchestrator の決定：入りえない capture のために他を追い出すのは害だけ．
  - それ以外は `try_reserve(n × frame)` を試みる（上限より小さいので上限で抑えられる）．失敗は「持たない」（確保失敗の message）．INFERRED：Linux では触れていない予約は物理 memory を食わない．
- block を書くたびに（store の lock を一度取って）「和 + この block > C」かを見る．超えるなら，読まれていない完了済みの `Held` を**完了順の古い方から**追い出して入るまで空ける．取っている最中のものは追い出さない．追い出しても入らなければ，**その block を書こうとした capture** を持たないことにする（buffer を捨て，hash と continuity は続ける）．
- channel 数が capture の途中で変わったときの不足分も `try_reserve` で伸ばし，失敗は「持たない」．`Vec` の不可謬な伸び（失敗で abort）は使わない．
- 完了したら store の entry は `Held(bytes)` か，持たなかった理由（必要な bytes と C，または確保失敗）．それから `sink.CAPTURE_WRITTEN` を今と同じに出す（P1）．

**追い出しの順（D5 改め）**：古い順．理由（owner）：新しい方を捨てると，`CaptureTimeout` で放った・読まれない capture が Session の終わりまで上限を塞ぐ．

- 退け：新しい方を持たない（D5 の最初の決定）．上の理由．
- 退け：予約を数える（下書き）．`h1 = request(0.6C)`，`h2 = request(0.6C)` で，r1 が block の途中で終わると同じ `process_block` の中で r2 が始まり（`lib.rs:293` の後に `:245`．VERIFIED），予約の瞬間に r1 が追い出され，読む機会の無かった `result(h1)` が「read sooner」と言われる（Fable M1）．
- 退け：client が出した request を数えて送る前に断る．client に状態が増え，放った handle が上限を塞ぐ問題が戻る（Fable M1）．
- 退け：Sink が `REQUEST_REJECTED` で断る．Run の event が client の読む時刻に依り P1 に反する．
- 退け：Session を終わらせる．monitoring の一度の遅れで実験が終わる．

**store の規律**（orchestrator の決定．risky step の素材）

- store の lock は葉（leaf）：持ったまま他の lock を取らない．Sink は自分の object の lock（origin/main `crates/ezsdr-kernel/src/coordinator/paced.rs:126`）の中で store の lock を取り，server の `read` は Run の lock を持たずに store の lock だけを取る（KC-24a の規律，origin/main `design/06-kernel-coordinator.md:329`）．
- 追い出した buffer と捨てた buffer は lock の中で外へ move し，**lock を放してから** drop する．`sink.CAPTURE_WRITTEN` の告知も lock の外で，store に entry を入れた**後**．
- `read` は `Held` の bytes を lock の中で move して外し，**lock の外で**書く（Fable F1）．書いている間の body は数えない．だから server の memory の山は最大で C（store）＋ 書いている一つ（≤ C）＋ 固定の buffer，**およそ 2 × C**（Fable m2．EA-10 と KL-48 に書く）．
- **memory は固定の大きさの塊（chunk）の pool**（orchestrator の決定 16，2026-11-20）．capture の bytes は塊の並び．追い出しと読み終わりは塊を pool の空きに返すだけで，OS には返さない（Session の終わりに返す）．塊は一度書いた後は再び初回の書き込み（page fault）を払わない．pool は C ＋ 書いている body を超えて増えない．送っている body の塊は書き終えるまで空きに返さない（C には数えない．山はおよそ 2 × C のまま）．形は `HostPool`（`crates/ezsdr-hostmem/src/lib.rs:17-49`：使い回す slot，縮まない）と同じ考え．塊の大きさと，新しい塊の確保・初回の書き込みを data thread の外で済ませる方法（例：request を受けた control path で）は S2 の test plan で決める．理由：手元（usrp-lnx03，THP madvise）で 1 GiB の解放 7〜40 ms（一度に），初回の書き込み 150〜700 ms（書く間に散る）を測った（numpy，3 回，2026-11-20）．data thread での解放は feed の 64 block を超えうる．
  - 退け：解放だけを別 thread に回す．初回の書き込みが残る．
  - 退け：リングバッファ．古い順の追い出しとは合うが，末尾をまたぐ capture と読んでいる領域の保護が面倒で，塊の pool で同じ効果が得られる．
- key は型つきの (output `Ident`, `ActionId`)．URI の文字列は端（store の lookup が一度 parse する）だけ（`mech:string-composed-identity`，`plan/maintenance/failure-mechanisms.md:14`）．
- 記録（tombstone）は失敗の道だけに残す：追い出した（`Evicted`）と持たなかった（必要な bytes と C，または確保失敗）．読まれた `Held` は entry ごと消す．`read` は「報告済み（今の `readable`）で store に entry が無い」を「一度読まれた」と答える（Fable n5 の一部を採る：読まれた分の記録を持たない）．報告されていない URI は今のとおり `not_found`（EA-13 の「報告された URI だけ」を `mem:` にも保つ）．

**値と決め方（D4）**：既定 1 GiB．100 MS/s・1 channel・cf32（800 MB/s）で約 1.34 s，200 MS/s で約 0.67 s，1 MS/s で約 134 s（計算）．`connect(capture_memory=...)` → `connect { capture_memory_bytes? }` → `connected { capture_memory_bytes }`．BindingProfile にも Manifest にも入れない（P1 で記録を変えないので再現に要らない）．

- 退け：host の空き memory の割合（host で変わり予め分からない）．上限なし（#97 の OOM が `mem:` で戻る，KL-43 (b)）．knob なしの固定値（高い rate の短い burst が `keep=True` に追いやられる）．

### (d) #97：`read` の streaming（D8）

今：`std::fs::read(path)` で全部を memory に載せ（`crates/ezsdr-server/src/lib.rs:304-305`），`Handled.body: Vec<u8>`（`lib.rs:71-78`）を `write_reply` が header の後に書く（`crates/ezsdr-server/src/transport.rs:152-159`）．client は `_read_exactly` で受け（`python/ezsdr/_client.py:135-143`），`samples()` が `.copy()` する（`python/ezsdr/session.py:185`）．VERIFIED．下書きの「client が body を 2 倍持つ」は Fable が Python 3.14 で再現しなかった（`BufferedReader.read(n)` が一つの object を返し，一つの要素の join はそれを返す）．今の山は body ＋ `samples()` の copy．D8 はそれでも正しい．

- **決定：server で streaming，client は一度だけ確保**．`Handled` の body を「bytes」か「file と長さ」にする．`read` は file を開いて `metadata().len()` を取る（ここで失敗すれば今のとおり error `io`，frame は壊れない）．`write_reply` は header（`body_bytes = len`）を書いてから 64 KiB の buffer で `len` だけ写す．file が途中で短い・読めないなら frame を完成できないので `Err` を返し（`transport.rs:144` の `?`），server は `Drop for Server`（`lib.rs:437-443`）で Manifest を書いて終わる（EA-15，Fable m5）．frame は変えない．`mem:` は「bytes」の形で store から move したものを書く．
- **client**：`result()` と `pieces()` は `artifact["size_bytes"]` の `bytearray` を `read` を**送る前に**確保し（Fable m4），返事の `body_bytes` が同じなら `readinto` で埋める．違うなら新しく確保して全部読み，`Error` を上げる（Session は続く）．1 channel は `np.frombuffer` で copy せず返し，多 channel は今のとおり転置の copy 一回．公開の `Session.read` は今のとおり `bytes` を返す（`python/ezsdr/session.py:817-818` の約束．Fable m4）．
- 退け：上限を超える `read` を断って path を読ませる．ssh 越しの client は file を読めず，`keep=True` の大きな capture が壊れる．
- 退け：streaming と上限の両方．streaming で server の害は消え，client の memory は利用者が頼んだ array．
- 退け：range read（`read { uri, offset, len }`）．protocol が増える．同じ host なら `np.memmap`．remote listener の phase で additive に足せる．
- request 側の `body_bytes` の上限（`transport.rs:119` の `ponytail:` の注）は今回は開いたまま（remote listener の phase，`plan/phase7/00-overview.md:106`）．

### (f) client の検査・`CaptureGap`・`pieces`（D5 改め，D7，R2，R4）

- **一つの判定**：artifact が「一つの array で返せる whole な capture」かを continuity と `partial` から決める関数を一つ置き（今の `samples()` の検査，`python/ezsdr/session.py:154-182` を移す），`result()`・`samples()`・`RunResult.capture`・`pieces()` の前検査がそれを使う．`result()` は `read` を送る**前**にこれを呼び，whole でなければ `CaptureGap`（D7，R4）．今の `samples()` の `Error` は `CaptureGap` になる（subclass なので `except Error` は今のまま効く）．`samples()`・`RunResult.capture` が `partial` も断るかは O1．
- **送る前の検査**（D5 改め）：`keep=False` の `request()` は `n × len(stream.channels) × 8 > capture_memory` なら送る前に `Error`．INFERRED：Python が読む radio は cf32 だけ（EA-17 の Ceiling，`design/16-easy-api.md:142`）なので 8 B．sc16（#74）なら保守的．
- **`pieces(artifact)`**（R2）：送る前に，continuity から各 piece の範囲と bytes の和を出し，`size_bytes` と合わなければ `Error`．合えば一つの buffer に読み（上の確保の規則），分ける．
  - map ごとに：run の始まり = `map.first` と各 gap の `start + len`．終わり = 各 gap の `start` と `map.end`．長さ 0 の run は捨てる（空の piece は返さない）．長さ 0 の gap は builder が map の先頭（SC-13，SC-30b）と末尾（SC-30c）にしか作らない（途中で `GAP_BEFORE` が時刻の跳びなしに来れば `GapFlagWithoutJump`：`crates/ezsdr-kernel/src/stream/continuity.rs:218-227,317-321`，`design/02-stream-contract.md:269-271`．VERIFIED）ので，それが作るのは空の run だけで，piece を余計に分けない．
  - byte の位置は HD-15 の file index（`design/10-host-data-path.md:60`：届いた tick `t` に対し `t − map.first − Σ g.len`（`g.start + g.len ≤ t` の gap））に `channels × 8` を掛けたもの．map をまたいで足す．
  - `Piece.start` は run の最初に**届いた** sample（`map.first` は先頭の gap を含むので，それではない．`crates/ezsdr-kernel/src/stream/continuity.rs:100-101`）．domain は map の domain なので，piece ごとに違いうる．
  - `invalid[c]` は piece の範囲から channel `c` の `valid` の segment を除いた残り（`valid` に `c` の list が無ければ piece 全体）．`channel_gaps` と同じものを指すはずで，builder が作った map で両者が一致することを試験で押さえる．
  - 値は変えない．1 channel の piece は buffer の view，多 channel は copy．
- 退け（R2 で置き換わった Fable M4 の推奨）：channel ごとに invalid な区間があれば `pieces` も `Error`．owner は印をつけて返す方を選んだ（値を変えずに隠さない）．
- 退け（D7 の代案）：`CaptureGap` に読んだ bytes をぶら下げる．例外に最大 1 GiB が付き，利用者が要らないかもしれない bytes を読む（Fable §3）．
- 退け：`result(h, raw=True)`．引数が増える．

### (g) child Run（D9）

child Run は simulation だけ（KC-37a，origin/main `design/06-kernel-coordinator.md:404`）．store が無いので，child の schedule の `sink.capture_in_memory` は HD-14 で断られ，child の Manifest に `mem:` は出ない．hardware で Spec を走らせる Python の入口が無いことは別 issue #122（D9 の owner の問いから）．

### (h) request ごとの遅れ（R3，Fable M3）

`count()`（`crates/ezsdr-server/src/lib.rs:445-449`）は数えるために全 event を clone する（`events(0)`，origin/main `crates/ezsdr-kernel/src/coordinator/stepping.rs:452-453`）．一つの `request()` + `result()` が少なくとも 4 回呼ぶ（`status` `lib.rs:200`，`submit` `:182` と `:465`，`wait_for` `:551`）．clone は `delivered` の lock の中で，data thread も毎 pass でその lock を取る（origin/main `stepping.rs:206,216`，`paced.rs:115`）．VERIFIED．control path は event を落とさない（origin/main `crates/ezsdr-kernel/src/event.rs:660-665`．VERIFIED）ので capture ごとに一つ増える．

- **決定（S3）**：`count` は server の `scan()`（`lib.rs:391-404`）を進めてから `scanned` を返す．`scan()` は `events(scanned)` で新しい尾だけを clone する（`delivered` は `Vec<Event>`（origin/main `crates/ezsdr-kernel/src/coordinator/state.rs:230`）なので `iter().skip` は O(1)．VERIFIED）．R3 の式「`scanned + events(scanned).len()`」だけだと `scanned` は `read` でしか進まず（`lib.rs:299,395-396`），読まない Session では尾が伸び続けるので，数えること自体が scan を進める（orchestrator の補い）．
- 残るもの：`wait_for` の返事が一つの event を取るのに `events(index)` で index から後ろを clone する（`lib.rs:551`）．index は待っていた event なので尾は短い．古い handle を後で `result()` したときだけ長い（INFERRED）．Kernel に一つ取る口を足すまで残す．

## 4. 規則と操作の列（AGENTS §9 step 1 の素材）

各列は短い操作の列と期待．「依る挙動」の印は VERIFIED（path:line）か INFERRED（理由）．数は C = 上限，frame = 1 channel cf32 の 8 B とする．試験の名前は案で，段の test plan が決める．壁時計で判定する試験は作らない（`mech:wall-clock-test-oracle`，plan 51 の規則：判定は試験の thread がいつ走るかに依らない）．

### S1（#97：streaming と client の一度の確保）

| # | 操作の列 | 期待 | 依る挙動 |
|---|---|---|---|
| Q1 | 1 channel の capture（S1 では今の file）→ `result()` | server は file を開き `metadata().len()` を header に，body を 64 KiB ずつ写す．client は `read` を送る前に `size_bytes` の buffer を確保し `readinto`．返る array は buffer と memory を共有する．bytes の hash が artifact の `hash` と一致 | `read` の道 `crates/ezsdr-server/src/lib.rs:298-308`，`write_reply` `transport.rs:152-159`，`size_bytes` は file が受けた bytes（HD-10，`design/10-host-data-path.md:50`）VERIFIED |
| Q2 | 2 channel の capture → `result()` | `(2, n)` の C-contiguous．copy は転置の一回だけ | 今の形 `python/ezsdr/session.py:184-185` VERIFIED |
| Q3 | 報告済みの capture の file を `set_len` で 3 GiB に伸ばし（sparse）`read` | server の `VmHWM` の増えが 64 MiB 未満．client が受けた bytes の数が 3 GiB．一つの test だけの試験 binary（process の `VmHWM` を他の試験が上げないように） | INFERRED：sparse な file の読みは 0 を返し page cache を食うが process の RSS には入らない．`VmHWM` は Linux の process の山 |
| Q4 | 報告済みの file を消してから `read` | error `io`．frame は壊れず，次の request が通る | `lib.rs:304-306` VERIFIED（EA-5 の `io`） |
| Q5 | `write_reply` に，実際の file より長い `len` の file の body を渡す（file が途中で短い） | `write_reply` が `Err`．`serve` を抜け，`Drop for Server` が Manifest を書く．client は「the server's output ended inside a body」の `ProtocolError`，Session は終わり | `transport.rs:144` の `?`，`lib.rs:437-443`，`python/ezsdr/_client.py:100-102,140-141` VERIFIED |
| Q6 | `result()` の buffer の確保が `MemoryError`（試験は確保の helper を差し替える） | `read` は送られない．次の呼び出しが通る．S4 以後の `mem:` なら bytes は残っていて，`result(h)` をもう一度呼べば読める | 送る前の失敗は exchange の外（`_client.py:100-102` の `except` は exchange の中だけ）VERIFIED |
| Q7 | 試験が `size_bytes` を偽った artifact で client の読みの道を呼ぶ | body を全部読み（新しい buffer），`Error`（大きさが違う）．Session は続く | 同上 |
| Q8 | `readinto` の途中で `KeyboardInterrupt` | 今の EA-16 のとおり両 pipe を閉じ，Session は終わる | EA-16 の「An interrupted call」（`design/16-easy-api.md:129-133`）VERIFIED |
| Q9 | 公開の `Session.read(artifact)` | 今のとおり `bytes` | `python/ezsdr/session.py:817-818` VERIFIED |

### S2（Sink と store．Sink と store の単体で，block を手で与える）

| # | 操作の列 | 期待 | 依る挙動 |
|---|---|---|---|
| Q10 | in-memory の request を 3 つ重ね，全部完了 | 連続（各々が前の終わりから）．URI は `mem:<output>/capture_<id>`．dir に file は無い．store に `Held` が 3 つで，各 bytes の hash が `ArtifactRef.hash` と一致 | 一つの queue（HD-10，`design/10-host-data-path.md:44`；`crates/ezsdr-sink-capture/src/lib.rs:181-183`）VERIFIED |
| Q11 | in-memory と file を交互に 4 つ重ねる | 連続．file は 2 個，`Held` は 2 個 | instance ごとの FIFO（origin/main `crates/ezsdr-kernel/src/coordinator/state.rs:43-49`），admission が `at` を保つ（origin/main `admission.rs:82-87`）VERIFIED．admission の後の dispatch が時刻つきの update を `at` まで留めないこと：INFERRED（Fable n1．今の重ねた timed request が順に届くことから） |
| Q12 | **P1**：同じ simulated Run（同じ seed）を C = 2^62 と C = 8 B で | Sink の event と `ArtifactRef`（id・uri・hash・size・partial・continuity）が全部同じ．違うのは store だけ | P1（§3 (b)）．Sink の出力が store に依らないこと |
| Q13 | r1・r2（各 0.4 C）が完了して読まれず，r3（0.4 C）を取る | r3 が 0.2 C まで書く間は何も追い出さない．和が C を超える block で r1（完了順で最も古い）が `Evicted`，r2 は `Held`．r3 は完了で `Held` | R1 |
| Q14 | **Fable M1**：r1・r2（各 0.6 C）を重ねる．(a) r2 が 0.3 C 書いた時点で r1 を take．(b) r2 が 0.5 C 書くまで take しない | (a) r1 の bytes が返る．(b) r2 の和が C を超える block で r1 が `Evicted`，r2 は `Held` | r1 の完了と同じ `process_block` で r2 が始まる（`lib.rs:293` の後に `:245`）VERIFIED |
| Q15 | **Fable M2 の A**：一つの store を二つの Sink（recorder）が共有し，各 0.6 C を同時に取る．完了済みは無い | block を与える順が決まっているので，C を越える block を書こうとした方が「持たない」（必要な bytes と C を記録）．buffer は捨て hash と continuity は続く．もう一方は完了で `Held`．両方の記録は C が十分なときと同じ．試験は持たない方を名指しで確かめる．device-paced の server 試験では「ちょうど一つが持たれない」を確かめる（pass の中の Sink の順，origin/main `paced.rs:91-95`，VERIFIED） | R1 |
| Q16 | **Fable M2 の B**：`Held` が二つある状態で，`n × frame > C` の request | 始めから持たない（確保も追い出しも無い）．前の二つは `Held` のまま．記録は変わらない | orchestrator の決定（§3 (c)） |
| Q17 | C = `u64::MAX` で `n × frame > isize::MAX` の request | `try_reserve` が失敗し「持たない」（確保失敗）．abort しない | `Vec::try_reserve` は `isize::MAX` を超える要求に `CapacityOverflow` を返す（標準 library の文書．INFERRED：本文は読んでいない） |
| Q18 | in-memory の capture の途中で recorder への targeted `Stop` | `partial: true` の `ArtifactRef`，`Held`（取った分）．始まっていない in-memory の request は「HD-11: discarded by a Stop for the Sink」の `REQUEST_REJECTED` | `lib.rs:187-199` VERIFIED（HD-11） |
| Q19 | in-memory の capture の途中で `stop(mode)` | `partial` の `ArtifactRef` を `stop` が返す．store に `Held`（Session の終わりで消える） | HD-13（`design/10-host-data-path.md:57`），`lib.rs:468-474` VERIFIED |
| Q20 | store の無い Sink に in-memory の request | `REQUEST_REJECTED`（`HD-14: this Run holds no capture in memory; use sink.capture`），`step` は `Ok`，artifact は無い | HD-14（`design/10-host-data-path.md:55`）VERIFIED |
| Q21 | Spec の output の `params` に `sink.capture_in_memory_samples` | `prepare` が `Rejected`（HD-9 のまま） | `lib.rs:398-411` VERIFIED |
| Q22 | in-memory の key に 0，または `Int` でない値 | `REQUEST_REJECTED`（`HD-14: sink.capture_in_memory_samples must be a positive Int`） | `lib.rs:185` の形 VERIFIED |
| Q23 | `sink.CAPTURE_WRITTEN` が出た時点の store | entry（`Held` か持たなかった記録）が既にある．告知の間 store の lock は持っていない | §3 (c) の規律 |
| Q24 | 同じ bytes の capture を二つ（無音） | entry が二つ，どちらも `Held` | key が (output, ActionId)（Fable F7） |
| Q25 | in-memory の capture の途中で contract の違う block | `partial` で終わり `Held`．その block は次の capture に回る | HD-10（`design/10-host-data-path.md:47`）VERIFIED |
| Q26 | store の take：`Held` を take → bytes，同じ key をもう一度 → 「無い」（一度読まれた）．`Evicted` → その記録．持たなかったもの → その理由 | 記録は失敗の道だけに残る | §3 (c) |

### S3（server：store の配線，`mem:` の `read`，`capture_memory`，`not_held`，`count()`）

| # | 操作の列 | 期待 | 依る挙動 |
|---|---|---|---|
| Q27 | `connect` に `capture_memory_bytes` 無し／`X` | `connected.capture_memory_bytes` が 1 073 741 824 ／ `X` | D4 |
| Q28 | raw の `submit` で `sink.capture_in_memory`，`wait_for` で `CAPTURE_WRITTEN`，その URI を `read`，もう一度 `read` | 一度目は bytes（hash が一致）．二度目は `not_held`（一度読まれた） | D6 |
| Q29 | 追い出された URI と，持たなかった URI を `read` | それぞれ §2.2 の message の `not_held`（追い出しは C を，持たなかったものは必要な bytes と C を言う） | R1，D5 改め |
| Q30 | 報告されていない `mem:rec/capture_999` を `read` | `not_found` | EA-13（`design/16-easy-api.md:80`），`lib.rs:300-302` VERIFIED |
| Q31 | **Fable F1**：device-paced（FakeDevice，manual clock）の Session で，大きな `mem:` の body を止まる writer に書いている間に data thread が次の capture を store に入れる．writer は store への insert で放される | deadlock しない．insert は書きの最中に起きる（書きは store の lock の外）．判定は insert という出来事で，時刻ではない | §3 (c) の規律；data thread が Sink を step する（origin/main `paced.rs:119-128`）VERIFIED |
| Q31a | **Fable m2**：Q31 と同じ道具で，C の `Held` を一つ move して止まる writer に書いている間に，data thread が新しい capture で store を C まで満たす | 何も追い出されない（書いている body は数えない）．その間の server の bytes はおよそ 2 × C．EA-10 と KL-48 の「およそ 2 倍」を押さえる | §3 (c) の規律 |
| Q32 | device-paced の Session で in-memory の request を重ね，届くたびに読む | 各 `read` がちょうど自分の bytes を一度返す．取りこぼし・二重は無い．store の lock を Run の lock と同時に持たない | KC-24a（origin/main `design/06-kernel-coordinator.md:329`）VERIFIED |
| Q33 | child Run の schedule に `sink.capture_in_memory` | child の event に `REQUEST_REJECTED`（HD-14，store が無い）．child の Manifest に `mem:` は無い．Session は続く | `run_child` は store を渡さない（`lib.rs:330`）．KC-37a（origin/main `design/06-kernel-coordinator.md:404`）VERIFIED |
| Q34 | capture を N 回した後の `status`・`submitted`・`waited` の `events` | どれも配られた event の数と等しい．各 event は `scan` で一度だけ clone される（費用は code review で見る．壁時計の試験は作らない） | R3；`events(from)` は `skip`（origin/main `stepping.rs:452-453`）VERIFIED |
| Q35 | 読まれていない in-memory の capture を残して `finish` | Manifest の `artifacts` に `mem:` の `ArtifactRef`（file のときと同じ形）．Session directory に file は無い | D3，`ending.rs:454-460`（origin/main）VERIFIED |
| Q36 | `mem:` の `read` の返事の書きが途中で失敗 | server は Manifest を書いて終わる（Q5 と同じ道） | Fable m5 |
| Q37 | frame の schema | `request_frame.v1.json`・`reply_frame.v1.json` を再生成（`Connect.capture_memory_bytes`，`Connected.capture_memory_bytes`，`ErrorKind::NotHeld`）．`ea_06_schema_freeze` が通る．version と `SCHEMA_CHANGELOG.md` は触らない | EA-6（`design/16-easy-api.md:39`），AGENTS §6 |

### S4（Python）

| # | 操作の列 | 期待 | 依る挙動 |
|---|---|---|---|
| Q38 | `capture(n)` | log の verb が `capture_in_memory`．array が返る．`sdr.dir` に `.sigmf-data` が増えない．Manifest の artifact の `uri` が `mem:` で始まる | D1 |
| Q38a | **Fable M1 を利用者の形で**：simulated Session で `h1 = rx.request(n1)`，`h2 = rx.request(n2)`（各 0.6 C），`rx.result(h1)`，`rx.result(h2)` | 両方とも値が返る．r1 は追い出されない | INFERRED：simulation の `wait_for` は合う event が配られた step で返る（EA-12 の `RunHandle::wait_for`，KF-2 は読んでいない）ので，r2 はそれまでに一 step 分しか書かない．試験は block の長さを 0.4 C より十分小さくとる |
| Q39 | `capture(n, keep=True)` | verb `capture`，file が一つ，`uri` が `file://` | D1 |
| Q40 | `n × channels × 8 > sdr.capture_memory` の `capture(n)`，続けて同じ n で `keep=True` | 一つ目は送る前に `Error`（`keep=True` と `capture_memory` を名指す）．`status` の `events` と log の長さが変わらない．`keep=True` は送られる | D5 改め；EA-16 の「送る前に断る，何も log に入らない」 |
| Q41 | simulation の fault（link drop）で gap のある capture → `result()` → `pieces(e.artifact)` | `result()` が `CaptureGap`（`.artifact`）．`read` は送られていない．続く `pieces()` が bytes を返す（store に残っていた証） | D7；`test_ea_17_a_capture_across_a_gap_is_refused` の fault の作り方 |
| Q42 | **R4**：`request(n)`，recorder への targeted `Stop`，`result(h)` → `pieces(e.artifact)` | `result()` が `CaptureGap`（partial）．`pieces()` が取った分の一つの piece | R4，HD-11 |
| Q43 | gap と restart（`rx.stop()`/`rx.start()`）のある capture の `pieces()` | piece が gap と SampleClock の変化で分かれる．各 `start` がその domain の最初に届いた sample．値が bytes と一致．末尾の長さ 0 の gap（SC-30c）は空の piece を作らない | R2；SC-30b/c（`design/02-stream-contract.md:269-271`）VERIFIED |
| Q44 | 分ける関数を手で作った artifact で呼ぶ：2 channel，channel 1 が 1 000 sample invalid | piece は一つ．`invalid[1] == [(b, b + 1000)]`，`invalid[0] == []`．値は変えない．builder が作った map では `valid` の補集合と `channel_gaps` が一致 | R2；`ChannelGap`・`valid`（`crates/ezsdr-kernel/src/stream/continuity.rs:63-73,94-95`）VERIFIED |
| Q45 | `mem:` の capture に `pieces()` を二度．file の capture に二度 | `mem:` の二度目は `not_held`（一度読まれた）．file は二度とも同じ | D6 |
| Q46 | **Fable m1**：`CaptureGap` の後，読まない capture を重ねて追い出させ，`pieces(e.artifact)` | `not_held`（追い出しの message） | D7「subject to D5 eviction」 |
| Q47 | `capture(n, timeout=小)` が `CaptureTimeout`，続けて `capture(n)` | `CaptureTimeout` に `.request` は無い．次の capture は通る．放った capture は取られ，追い出されるまで持たれる | D6；EA-17 の「still served first」（`design/16-easy-api.md:142`）VERIFIED |
| Q48 | `result(h)` を二度 | 二度目は `not_held`（一度読まれた） | D6 |
| Q49 | `connect(capture_memory=X)`；`0`・`-1`・`True`・`"1"` | `sdr.capture_memory == X`．他は server を起こす前に `Error` | D4，O2 |
| Q50 | `with sdr.aligned(t):` の中で `request(n)` と `request(n, keep=True)`，block の後で `result` | 両方 `t` から．memory と file | EA-16 の `aligned`（`design/16-easy-api.md:103`）VERIFIED |
| Q51 | `mem:` の `result()` で確保が `MemoryError`，続けて `result(h)` | 一度目は送らずに `MemoryError`，二度目は bytes を返す | Fable m4 |
| Q52 | Python 3.9 の container で S4 の試験一式 | 通る（keyword-only 引数，例外 class，`readinto`，`bytearray` の `np.frombuffer` は 3.9 にある） | Fable n8（INFERRED．3.9 は未実行．command は handoff §5） |

### 実機の確かめ（Fable m3．S3 の後）

| # | 操作の列 | 期待 | 依る挙動 |
|---|---|---|---|
| Q53 | 受信だけ（送信なし），1 channel，200 MS/s と 100 MS/s，C = 1 GiB．(a) 基準：0.4 C の in-memory の capture を重ね，届くたびに読む（追い出し無し）．(b) 同じ capture を読まずに重ね続ける（以後の capture ごとに data thread で約 0.4 C の追い出しと確保）．各 cell 10 回以上，60 s | 判定を先に決める：(b) の `link_drop` の gap の数が (a) を超えない（(a) が 0 なら (b) も 0）．塊の pool（§8.2 の 16）で (b) も gap を増やさないことの確かめ．超えたら塊の確保と返しの道を調べる | INFERRED：1 GiB の解放・初回の書き込みは数十 ms（Fable m3，未測定）．feed は `drop_oldest` 64 block（`crates/ezsdr-server/src/catalogue.rs:222`）．S3 の build が要る．2026-11-27 の bench（origin/main `plan/phase8/50-remaining-work.md:65`）に S3 が間に合えばそこで，間に合わなければ S3 の後の最初の bench．それまで KL-48 はこの点を INFERRED と書く |

## 5. 改める文（design/ と KNOWN_LIMITATIONS）

AGENTS §2：KL の文は振る舞いを変える commit と同じ commit で．だから段の列を付ける．spec 10 と 16 はまだ本文に履歴を持つ（DA-3 の順番待ち）ので，`plan/maintenance/tools/rules.py` は本文の履歴の印（`(owner, 2026-…)`，`#73`，`(issue #97)` など，`rules.py:45-46` の `MARKER`）と 1 500 字を超える行の**数**が増えないことだけを求める（`rules.py:16,124-125`）．だから：本文に issue 番号や owner の日付を書かず，閉じの `## Changes` 表（`design/10-host-data-path.md:137`，`design/16-easy-api.md:241`）に一行ずつ足す．今 1 500 字に近い行（HD-14 `design/10:55` 1 385 字，EA-5 `design/16:38` 1 437 字，EA-10 `design/16:69` 1 382 字）には足さず，新しい行（sub-bullet）に書く．spec が引く新しい試験の名前は同じ commit に在ること（`rules.py` が見る）．version は一つも動かさない．

| 規則 | path:line | 段 | 変更 |
|---|---|---|---|
| EA-13 | `design/16-easy-api.md:80` | S1 | `file://` は 64 KiB の buffer で流して送る．header の後で書けなくなったら（短い file）返事は完成せず，server は EA-15 の道で終わる（Fable m5） |
| EA-15 | `design/16-easy-api.md:82` | S1 | 文は変えない（「a reply it cannot write」が body の途中の失敗を含む）．確かめだけ |
| EA-16 | `design/16-easy-api.md:116,129-133` | S1 | `read` の行は `bytes` のまま．`result()` は送る前に `size_bytes` の buffer を確保する（確保の失敗は送らず Session を保つ）．body の大きさが違えば読み切って `Error` |
| EA-17 | `design/16-easy-api.md:142` | S1 | 「reads the artifact's bytes」に：一つの buffer に読み，1 channel は copy しない |
| HD-6 | `design/10-host-data-path.md:35` | S2 | 行は 1 094 字で，`KeyDecl` と `VerbDecl` を一つずつ足すと 1 500 字に近づくので，足す分は新しい行に．`keys` に `sink.capture_in_memory_samples`（`int`，coercible でない，`reject`，`block_boundary`），`verbs` に `capture_in_memory` → `UpdateParameter { key: sink.capture_in_memory_samples }`．`ezsdr_sink` の定数．version 1.1.0 のまま（§4 の見出し `:33` も） |
| HD-7 | `design/10-host-data-path.md:39` | S2 | 変えない（1.2.0，`artifact_kinds: [sink.capture]` のまま） |
| HD-8 | `design/10-host-data-path.md:40` | S2 | selector は今のまま．store は builder で host（server）から来て，binding からは来ない |
| HD-9 | `design/10-host-data-path.md:41` | S2 | 文は変えない（own capture は file のまま．新しい key は `params` で断られる）．試験の名前を一つ足す |
| HD-10 | `design/10-host-data-path.md:42-53` | S2 | `:44` request はどちらの key でも同じ queue．`:46` id は同じ．`:48` file は `sink.capture_samples` の capture だけ．`:50` in-memory の capture の `uri` は `mem:<output id>/<artifact id>`，hash と `size_bytes` は記録した bytes 全部の上．新しい sub-bullet：store（host が渡す，Session ごと），上限の数え方（書いた bytes，recorder の和），古い順の追い出し（完了済みで読まれていないもの，要るときだけ），持たない場合（追い出しても入らない，始めから上限を超える，確保の失敗），記録は持てても持てなくても同じ |
| HD-11 | `design/10-host-data-path.md:54` | S2 | 「an `UpdateParameter` on `sink.capture_samples` is a request」を両方の key に |
| HD-13 | `design/10-host-data-path.md:57` | S2 | partial の in-memory の capture も同じ（bytes は store に） |
| HD-14 | `design/10-host-data-path.md:55` | S2 | 断りの理由に「store が無い」（`HD-14: this Run holds no capture in memory; use sink.capture`）と in-memory の key の値．行が 1 500 字に近いので HD-10 の新しい sub-bullet か新しい行に |
| HD-15 | `design/10-host-data-path.md:58` | S2 | 「Every capture is a SigMF Recording」→ file に書く capture は．`mem:` に meta は無い |
| HD-16 | `design/10-host-data-path.md:67` | S2 | in-memory の capture は store に入れた（または持たないと決めた）後に告知する．「after the data file is closed and its metadata written」は file の capture のこと |
| EA-4 | `design/16-easy-api.md:23-37` | S3 | `:27` `connect { …, capture_memory_bytes? }` と `connected { …, capture_memory_bytes }`．`:33` `read` は `mem:` も |
| EA-5 | `design/16-easy-api.md:38` | S3 | kind `not_held`：一度読まれた，追い出された，持たなかった（取っている最中の和，始めから上限超え，確保の失敗）．新しい行に |
| EA-6 | `design/16-easy-api.md:39` | S3 | 文は変えない．schema を再生成（形の変化），version は上げない |
| EA-7 | `design/16-easy-api.md:43` | S3 | `assemble` が store を受ける（Session の `connect` だけが渡し，`run_child` と acceptance rig は渡さない）．今の文の `assemble(profile, inputs, implementations)` は既に `open_device` を落としているので，引数を正しく書き直す |
| EA-10 | `design/16-easy-api.md:69` | S3 | `capture_memory_bytes`：既定 1 GiB，Session の recorder の和，読みの途中は server がおよそ 2 倍まで持つ（Fable m2），`connected` が返す，BindingProfile にも Manifest にも無い．新しい行に |
| EA-13 | `design/16-easy-api.md:80` | S3 | `mem:` URI も報告されたものだけを読む．一度きり．追い出し・持たなかった・一度読まれたは `not_held`．書きは store の lock の外 |
| EA-14 | `design/16-easy-api.md:81` | S3 | child Run の Sink には store が無い（in-memory の request は HD-14） |
| KL-48（新） | `KNOWN_LIMITATIONS.md` の「Python client and Sessions」，KL-43（`:315-320`）の後 | S3（S4 で改める） | §5.1 の文 |
| KL-27 | `KNOWN_LIMITATIONS.md:338-343` | S3 | 一行：`mem:` の capture を input に並べると，scheme の検査（origin/main `crates/ezsdr-kernel/src/coordinator/pipeline.rs:389`）は通り，hash で bytes を探すので，その bytes を `waveform()` で渡す（Fable n3） |
| EA-16 | `design/16-easy-api.md:94,107,108,116,118,119` と新しい行 | S4 | `connect(…, capture_memory=None)` と `Session.capture_memory`（O2）；`capture(n, at=None, timeout=None, *, keep=False)`；`request(n, at=None, *, keep=False)`；`read` は `mem:` で一度きり；`samples()` の断りは `CaptureGap`（O1）；例外に `CaptureGap`（`.artifact`）；新しい行 `Session.pieces(artifact) -> list[Piece]` と `Piece(start, samples, invalid)`；送る前の検査（`connect(capture_memory=)` の値） |
| EA-17 | `design/16-easy-api.md:142` | S4 | 既定で `sink.capture_in_memory`（`sink.capture_in_memory_samples = n`），`keep=True` で `sink.capture`．`keep=False` の送る前の上限の検査．「refused with `Error`」→ `read` の前に `CaptureGap`，`partial` も（R4）．`pieces` の分け方と channel ごとの `invalid`（R2）．Ceiling の cf32 は `pieces` と検査の 8 B にも |
| EA-18 | `design/16-easy-api.md:143` | S4 | package が名指してよい名前に `sink.capture_in_memory` と `sink.capture_in_memory_samples` |
| KL-5 | `KNOWN_LIMITATIONS.md:45-51` | S4 | When の「Captures written to disk」は `keep=True` のときだけ．memory の capture の速さは data thread の hash に縛られうる（INFERRED，未測定．bench の示した以上を約束しない，Fable n7） |
| KL-21 | `KNOWN_LIMITATIONS.md:280-285` | S4 | 書き直し．Symptom：`result()`/`capture()` が `ezsdr.CaptureGap`．When：gap，invalid な channel の区間，restart，channel 数の変化，partial．Workaround：`sdr.pieces(e.artifact)`（channel ごとの `invalid` を見る．memory の capture は一度だけ読める，追い出される前に）．Status：Ceiling のまま |
| KL-22 | `KNOWN_LIMITATIONS.md:287-292` | S4 | 中身は変えない．Workaround に `Piece.start` を一言足してよい |
| 変えない | origin/main `design/04-run-and-session.md:183`（RS-20），`:220`（RS-34），`:308`（RS-44）；`design/06-kernel-coordinator.md:226`（KC-9），`:404`（KC-37a）；`design/07-radio-model.md`（`grep -n "sink.capture\|capture(" design/07-radio-model.md` が空）；Vision §50・§51（`design/vision/09-provenance-validation-and-ownership.md:14,24-25`），invariant 27；KL-43 | — | P1 がこれらを守る．§51 の SigMF は "should" |

### 5.1 KL-48 の文

S3 で入れる形（S3 の時点で利用者が届くのは raw の `sink.capture_in_memory` と `Session.read` だけ）：

```
### KL-48 Captures in memory: a cap, eviction, read once
- **Symptom:** `Session.read` of a capture requested with `sink.capture_in_memory` raises `ServerError` with kind `not_held`; a Session's action log, events and Manifest `artifacts` grow by one entry per capture (about 1–2 KB each, INFERRED).
- **When:** unread captures in memory pass the Session's capture memory (`connect`'s `capture_memory_bytes`, default 1 GiB, summed over its recorders): the oldest unread ones are evicted to make room; captures in progress together need more than that, or one request alone does: that capture is not held; a capture in memory is read a second time; a capture not read before the Session ends is lost (the Manifest keeps its record, hash and continuity). A request that timed out, or one submitted raw, is still served first and fills memory like any other (HD-10). While a capture is written to the client the server can hold about twice the cap. Freeing an evicted capture runs on the data thread of a device-paced Session; whether that adds receive gaps at high rates is not yet measured (INFERRED).
- **Workaround:** read each capture soon after its `sink.CAPTURE_WRITTEN`; request with `sink.capture`, which writes a file that can be read again; raise `capture_memory_bytes` at `connect`; a targeted `Stop` for the recorder discards requests that have not started (HD-11); for monitoring over many hours, open a new Session from time to time.
- **Evidence:** EA-5, EA-10, EA-13 (`design/16-easy-api.md`); HD-10 (`design/10-host-data-path.md`); design note n73.
- **Status:** Ceiling (owner, 2026-11-20: oldest-first eviction, read once, no discard operation).
```

S4 で改める所：Symptom の主語に「`capture()`, `result()` or `sdr.pieces`」を足す（S4 から `capture()` の既定が memory）．Workaround の `sink.capture` を「`keep=True` (`sink.capture`)」に，`capture_memory_bytes` を「`connect(capture_memory=...)`」に．Evidence に EA-16，EA-17．

## 6. 段取り

S1 は単独で先に着地できる．S2・S3 は一つの risky step の二つの小段（Q10〜Q37 を共有し，AGENTS §9 step 1 の「挙動は一か所に」に合う）で，test plan（step 2）と Sol の plan レビュー（step 3）を一度に．S4 は S3 の後．

| 段 | files | 足す試験（案） | risky か |
|---|---|---|---|
| **S1**（#97） | `crates/ezsdr-server/src/lib.rs`（`Handled` の body を bytes か file に，`read`），`src/transport.rs`（`write_reply` の 64 KiB の写し），`tests/protocol.rs`（`read` helper `:163-169` が `handled.body` を使う），新しい試験 file 一つ（Q3 を自分の process で）；`python/ezsdr/_client.py`（読む先の buffer を受ける `_call`），`python/ezsdr/session.py`（`result()` の確保と `readinto`，1 channel の view）；`python/tests/test_easy_api.py`；EA-13・EA-16・EA-17 の文 | Q1〜Q9：`ea_13_a_large_file_is_streamed_through_a_fixed_buffer`（Q3），`ea_13_a_file_shorter_than_its_header_ends_the_session_with_its_manifest`（Q5，`write_reply` の単位），`test_ea_17_a_one_channel_capture_is_not_copied`，`test_ea_16_a_failed_allocation_sends_no_read`，`test_ea_13_a_body_of_another_size_raises_and_keeps_the_session`，`test_ea_16_an_interruption_while_the_body_is_read_into_the_buffer_abandons` | **risky でない**：queue・deadline・identity・寿命・FFI・fake の境のどれも変えない．途中の失敗は既存の EA-15 の道．self-review の checklist は AGENTS §9 step 5 の標準 |
| **S2** | `crates/ezsdr-sink/src/lib.rs`（定数，`KeyDecl`，`VerbDecl`），`crates/ezsdr-sink/tests/sink_vocabulary.rs`（`hd_06_vocabulary` が key 一つ・verb を固定している：`:31-33`），`crates/ezsdr-sink-capture/src/lib.rs`（行き先 file/memory，store の型，`with_store`，上限の検査と追い出し，`try_reserve`，`mem:` URI，store が無いときの HD-14），`crates/ezsdr-sink-capture/tests/sink_capture.rs`；HD-6/8/9/10/11/13/14/15/16 の文 | Q10〜Q26：store の純粋な単体（数え方，完了順の追い出し，記録，take の一度）と Sink の試験．名前の案：`hd_10_requests_in_memory_and_to_files_share_one_queue`，`hd_10_the_record_does_not_depend_on_the_cap`（Q12），`hd_10_eviction_takes_the_oldest_unread_when_memory_is_needed`，`hd_10_a_pipelined_capture_does_not_evict_before_it_needs_memory`（Q14），`hd_10_captures_in_progress_over_the_cap_drop_the_one_that_crosses`（Q15），`hd_10_a_request_over_the_cap_is_never_held_and_evicts_nothing`（Q16），`hd_10_a_failed_reservation_is_not_held`，`hd_14_a_run_without_a_store_refuses_an_in_memory_request`，`hd_16_an_in_memory_capture_is_announced_after_the_store_has_it` | **risky**：bytes の寿命，store の持ち主，artifact の identity（key），data thread と control thread が共有する状態 |
| **S3** | `crates/ezsdr-server/src/lib.rs`（`connect` で store を作り `Live` に持つ，`read` の `mem:`，`count` が `scan` を進める，`not_held`），`src/catalogue.rs`（`assemble` の store 引数），`src/protocol.rs`（`Connect`・`Connected`・`ErrorKind`），`crates/ezsdr-acceptance/src/rig.rs:200,243`（`None` を渡す），`schemas/server/{request,reply}_frame.v1.json`（`EZSDR_UPDATE_SCHEMAS=1` で再生成），`tests/protocol.rs`；EA-4/5/6/7/10/13/14，KL-48，KL-27 | Q27〜Q37：`ea_10_capture_memory_defaults_and_is_echoed`，`ea_13_a_capture_in_memory_is_read_once`，`ea_13_an_evicted_capture_is_not_held`，`ea_13_a_capture_never_held_says_why`，`ea_13_an_unreported_mem_uri_is_not_found`，`ea_13_the_store_lock_is_not_held_while_a_body_is_written`（Q31，FakeDevice の manual clock，出来事で判定），`ea_13_device_paced_reads_and_inserts_lose_and_duplicate_nothing`（Q32），`ea_14_a_child_in_memory_capture_is_refused`，`ea_04_event_counts_after_many_captures`，`ea_06_schema_freeze`（再生成） | **risky**：Session の寿命と store，data thread と control thread の共有，protocol の形 |
| **S4**（#73） | `python/ezsdr/session.py`（`keep=`，送る前の検査，一つの判定，`CaptureGap`，`pieces`/`Piece`，`connect(capture_memory=)`，`Session.capture_memory`），`python/ezsdr/__init__.py`（`CaptureGap`・`Piece` を export），`python/tests/test_easy_api.py`；EA-16/17/18，KL-5・KL-21（・KL-22）；Python 3.9 の container の run | Q38〜Q52．分ける関数は純粋な試験（手で作った artifact）で，server を通す試験は Q41〜Q46 だけ（AGENTS §9 Maintainability 3）．**既存の試験の監査表**（AGENTS §9 step 1）：`test_ea_16_an_interruption_while_the_reply_body_is_read_abandons`（`python/tests/test_easy_api.py:1863-1869`）は `capture(100)` の後に同じ artifact を `sdr.read` するので，既定が memory になると二度目の読みで `not_held`（body が無く割り込みが起きない）→ `keep=True` に直す．`test_ea_17_a_capture_across_a_restart_is_refused`（`:744-761`）は raw の `sink.capture` で，`CaptureGap` は `Error` の subclass なので通る．Manifest の artifact の continuity を見る試験（`:112`，`:597`，`:613`，`:736`）は `mem:` でも同じ形（INFERRED，run で確かめる）．examples：`python/examples/minimal.py:11`，`bench_loopback.py:29,32`（N = 20 000，160 kB），`sweep.py:34`（`RunResult.capture`，file）はどれも file を読まない．`plan/`・`bench/` に Python の呼び手は無い（`grep -rn "\.capture(\|\.request(\|\.result(" python/examples python/tests plan bench --include=*.py`） | **risky でない**：API の意味は step 0 と owner の判断で決まり，状態は client に増えない（`capture_memory` を読むだけ）．self-review は標準 |

各段の mutation 行（Rust は字句，他は完全一致の部分文字列）：S1 は buffer の写しの長さ・確保の順，S2 は書いた bytes で数えること・完了順・取っている最中を追い出さないこと・始めから超える capture・`try_reserve`・告知の順，S3 は `not_held` の三つの原因・報告済みの検査・`count` の `scan`，S4 は送る前の検査・読む前の `CaptureGap`・`partial`・piece の境と `invalid`．

着地：AGENTS §6 の landing の検査（`land.sh`）で，commit の件名が名指す issue に `mech:` か `mech: none — <理由>` が要る．#97（bug）は orchestrator が着地のときに決める（案：`mech: none — a whole-file read with no streaming; no ledger mechanism fits`）．#73 は enhancement（案：`mech: none — an enhancement`）．

## 7. Fable 5.2 のレビューの処置（orchestrator，2026-11-20）

| 指摘 | 処置 | 理由（一行） |
|---|---|---|
| M1 予約の勘定で重ねた capture が読む前に追い出される | 採る（owner R1） | 書いた bytes で数え，要るときだけ追い出す（§3 (c)，Q14） |
| M2 追い出すものが無い溢れの規則が無い | 採る（owner R1）．orchestrator が補う：一つで上限を超える capture は始めから持たず，何も追い出さない | 越えた capture を持たない，専用の message，`try_reserve`（Q15〜Q17） |
| M3 `count()` の clone で monitoring が遅くなる | 採る（owner R3）．orchestrator が補う：数えること自体が scan を進める | R3 の式だけでは読まない Session で尾が伸びる（§3 (h)） |
| M4 `pieces` に channel ごとの有効性の規則が無い | 規則は採る．Fable の推奨（`Error`）は owner R2 が置き換えた | `invalid` の list で印をつけ，値は変えない（Q44） |
| m1 `CaptureGap` と `pieces` の間の追い出し | 採る（変更なし） | owner が D7 で「追い出しの対象」と受けた．message が何が起きたか言う．Q46 と KL-48 |
| m2 server の山はおよそ 2 × 上限 | 採る（文で） | EA-10 と KL-48 に書く．書いている body を数える仕組みは足さない |
| m3 data thread での解放・初回の書き込み | 採る（実機の確かめ，owner R1）．さらに塊の pool を始めから（§8.2 の 16） | Q53：基準つき，各 cell 10 回以上，判定を先に |
| m4 client は `read` の前に確保する | 採る | `result()` と `pieces()`．公開の `Session.read` は `bytes` のまま．下書きの「2 倍」の主張は取り下げ |
| m5 流す途中の失敗で Session が終わる | 採る | EA-13 に書く．一つの frame の中では他に無く，詰め物は黙った破損（Q5，Q36） |
| n1 混ぜた順は安全 | 採る（記録） | Q11 の「依る挙動」 |
| n2 `partial` が短い array で黙って返る | 採る（owner R4） | `result()` は `CaptureGap`．`samples()`・`RunResult.capture` は O1 |
| n3 `mem:` URI と input の衝突・Session 間の重複・input に並べたとき | 採る | KL-27 に一行（§5） |
| n4 再生と invariant 27 | 採る（記録） | P1 の確かめ．hardware の Run はもともと再生できない |
| n5 URI ごとの記録を持たずに `not_held` を答える | 一部を採る | 読まれた分は記録しない．ただし owner の三つの message（D5 改め，D6，R1）を分けるため，追い出しと持たなかったものの記録は持ち，EA-13 の「報告された URI だけ」は `mem:` にも保つ（§3 (c)） |
| n6 放った request が先に取られ，役に立つ capture を押し出す | 採る | KL-48 に一文．対処は targeted `Stop`（HD-11） |
| n7 disk が無いと data thread の hash が速さの上限 | 採る | KL-5 の書き直しで bench の示した以上を約束しない |
| n8 Python 3.9 | 採る | S4 で 3.9 の container を走らせる（Q52） |
| §4 の文の表 | 採る | §5 に取り込んだ．下書きの §5 の古い行（`sink.take`，`Error.data`，`CaptureTimeout.request`）は捨てた |
| §6「試験が既定の道で `file://` に依らない」は確かめていない | 確かめた | 一つ依る：`test_ea_16_an_interruption_while_the_reply_body_is_read_abandons`（S4 の監査表） |

## 8. 残る判断と orchestrator の決定

### 8.1 owner に残る点（どちらも利用者に見える．S4 の brief までに）

- **O1 `partial` を `samples()` と `RunResult.capture` でも断るか**（**決定 2026-11-20：推奨のとおり断る**）．R4 は `result()` について決めた．一つの判定（§3 (f)）を全部で使うと，child Run の own capture が時間切れで短いとき，`RunResult.capture` が今の短い array の代わりに `CaptureGap` を上げる（file なので `sdr.pieces(e.artifact)` で取れる）．推奨：断る（「whole な capture」の意味を一つにし，短い array を黙って返さない）．損：`RunResult.capture` の今の振る舞いが変わる（利用者は owner だけ）．代案：`partial` の検査は `result()` だけに置く（判定が二通りになる）．
- **O2 `Session.capture_memory` と値の検査**（**決定 2026-11-20：推奨のとおり**）．D4 は「`connected` が返す」．推奨：`sdr.capture_memory`（読むだけの int）で見せ，`connect(capture_memory=)` は正の int（`bool` は不可）だけを受け，それ以外は server を起こす前に `Error`．server 側は任意の `u64` を受ける（0 なら何も持たない）．代案：属性は足さず `connected` の値は内部だけ．

### 8.2 orchestrator の決定（「decided by the orchestrator」．owner はどれも覆せる）

1. 一つで上限を超える capture は始めから持たず，何も追い出さない（M2 の B）．
2. 予約は `try_reserve(n × frame)` で，数えるのは書いた bytes．途中で足りない分も `try_reserve`．
3. 記録は失敗の道（追い出し，持たなかった）だけ．「報告済みで store に無い」は一度読まれた（n5 の一部）．
4. `mem:` の `read` も報告済みの URI に限る（無ければ `not_found`）．
5. store の key は (output, ActionId)．URI は端で一度 parse．
6. store の lock は葉．追い出した buffer は lock を放してから drop．告知は insert の後，lock の外．`read` の書きも lock の外．
7. `count` は `scan` を進めて `scanned` を返す（R3 の補い）．
8. 公開の `Session.read` は `bytes` のまま．`readinto` は `result()` と `pieces()` だけ．
9. body の大きさが `size_bytes` と違えば読み切ってから `Error`（Session を保つ）．
10. piece はどの gap でも分け，空の piece は捨てる（長さ 0 の gap は map の両端にしか無いので結果は同じ）．`result()` は今のとおりどんな gap でも `CaptureGap`．
11. `Piece.invalid` は `valid` の補集合から（`channel_gaps` との一致を試験で押さえる）．
12. client の上限の message の接頭は `EA-17:`．`not_held` の message は §2.2 の形（確保失敗の message を一つ足す）．
13. S2 と S3 は一つの risky step の二つの小段．
14. Q53 は S3 の後の最初の bench（2026-11-27 に間に合えばそこ）．塊の pool（16）で gap が出ないことの確かめ．
16. store の memory は塊の pool（§3 (c)）．Fable m3 の「gap が出たら直す」を，手元の測定から始めの設計に入れる．
15. `wait_for` の `events(index)` の尾の clone（`lib.rs:551`）は今回は残す．

## 9. 範囲の外

- `sc16` を host で（#74）．`pieces`・`samples()`・送る前の検査は cf32 の 8 B を前提にする（EA-17 の Ceiling）．
- hardware で Spec を走らせる Python の入口（#122．D9 で child Run が simulation だけと確かめたことから）．
- request 側の `body_bytes` の上限（`crates/ezsdr-server/src/transport.rs:119`，remote listener の phase）．
- range read，`mem:` の二度読み，捨てる op，二つ目の client が同じ capture を読むこと．
- 上限を BindingProfile や Manifest に記録すること（server 所有の profile の phase で additive に）．
- 何時間もの monitoring での log・event・Manifest の伸び（1 capture 約 1〜2 KB，INFERRED）と，`wait_for` の尾の clone（§3 (h)）．KL-48 に書き，直しは要るときに別 issue．
- memory の capture の速さ（data thread の hash，Fable n7）の測定．

## 10. 後でできなくなること

- `mem:` の capture を二度読む，または別の client が読むこと．変えるには解放の op（退けた `remove_result` の形）が要る．
- Manifest に「client が samples を受け取ったか」を書くこと（P1）．
- `mem:` の意味を「Run の外に残らない bytes」以外にすること（input と capture の両方がこの意味に乗る）．
- 大きな `read` を途中でやめて Session を続けること（一つの frame．range read を足せばできる）．
- wire の `sink.capture` の既定を memory に変えること（Spec の schedule を巻き込む）．

## 11. 証拠（本文で引いたものの他）

- VERIFIED：`sink.capture` は単一 key の `UpdateParameter` に compile される（`crates/ezsdr-kernel/src/session.rs:367-384`）．
- VERIFIED：Sink は `sink.capture_samples` だけを request として受け（`crates/ezsdr-sink-capture/src/lib.rs:181-186`），file を作って `file://` を書き（`:299-321,328-348`），`stop` が完了した `ArtifactRef` を返す（`:468-474`）．`done` は Session の終わりまで全 `ArtifactRef` を持つ（`:345`）．
- VERIFIED：Sink を作るのは server の catalogue（`crates/ezsdr-server/src/catalogue.rs:115`），`assemble` は `open_device` を受ける（`:90`），呼ぶのは `connect`（`crates/ezsdr-server/src/lib.rs:245`），`run_child`（`:330`），rig（`crates/ezsdr-acceptance/src/rig.rs:200,243`）．
- VERIFIED：`read` は `file://` だけを剥がし報告済みの URI だけを読む（`crates/ezsdr-server/src/lib.rs:298-308`），報告は `scan()`（`:391-404`），child の Manifest から（`:369`）．
- VERIFIED：`count()` は全 event を clone する（`crates/ezsdr-server/src/lib.rs:445-449`；origin/main `crates/ezsdr-kernel/src/coordinator/stepping.rs:452-453`）．
- VERIFIED：`sink.CAPTURE_WRITTEN` と `sink.REQUEST_REJECTED` は `emit_control` で，control path は本体を落とさない（origin/main `crates/ezsdr-kernel/src/event.rs:660-665`）．だから `result()` が URI を知る道は ring の溢れで切れない．
- VERIFIED：`samples()` の今の断り（`python/ezsdr/session.py:154-185`），`capture()` は handle を返さない（`:406-415`），`CaptureTimeout` は何も持たない（`:45-46`），`RunResult.capture` は読んでから `samples()`（`:601-607`）．
- 計算：1 GiB は 100 MS/s・1 channel・cf32 で約 1.34 s，200 MS/s で約 0.67 s，1 MS/s で約 134 s．
- 確かめていないこと：試験も build も走らせていない．Python 3.9 は未実行．Fable の時間の見積もり（clone 1 件 2 µs，1 GiB の解放 数十 ms）は未測定．`Vec::try_reserve` の `CapacityOverflow` は標準 library の文書による（本文は読んでいない）．
