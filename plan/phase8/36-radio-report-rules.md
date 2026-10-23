# 設計メモ 36：radio Provider が報告する規則を，語彙の 1 か所で決める（#51，D2）

2026-10-22，orchestrator（Opus）．Phase 8 の 1b-6 の次の段階（handoff §2）．AGENTS.md §6 の three strikes による設計メモ．決定 1〜5 は owner が同日に推奨どおり決めた．Sol 6.1 のレビュー（修正して採用，major 3・minor 3，すべて採用）を入れた版（§7）．

## 1. なぜ，何を

`mech:radio-rule-implemented-twice`（[failure-mechanisms](../maintenance/failure-mechanisms.md)，9 件）は，時刻の規則については spec 22 VH-7（`ezsdr_radio::timeline`）で答えたが，**Provider が何を報告するか**の規則は開いたまま（[00-overview](00-overview.md) P4 の (2)：「イベントの報告規則も語彙で 1 か所に決め，両 Module がそれを呼ぶか」）．今 MockRadio と UHD Module で割れているのは次の 2 つ（どちらも 2026-10-22 に HEAD `437d2d9` のコードで VERIFIED）．

**#51：受け付けた後に捨てた送信の Action を報告するか．** 両 Provider とも，捨てる関数の bool 引数を呼び出し箇所が決める（MockRadio `stop_tx(.., emit_command_rejected)`・`cancel_later(.., emit)`，`crates/ezsdr-mock-radio/src/lib.rs:1112, 1357`；UHD `Tx::stop_now(.., emit)`，`crates/ezsdr-radio-uhd/src/provider/tx.rs:686`）．spec は RM-16 で「`Stop` は held burst を取り消す」と書くだけで，報告を決めていない．

| 捨てる理由 | MockRadio | UHD Module |
|---|---|---|
| `Stop`（`<id>/tx/s<k>` か `<id>`） | 記録だけ（`lib.rs:1458-1459`） | **`COMMAND_REJECTED`**（`tx.rs:191`） |
| `Provider::stop` | 記録だけ（`lib.rs:1142, 1145`） | 記録だけ（`tx.rs:872`） |
| loss | 記録だけ（`"MR-20: device lost"`，`lib.rs:2068`） | uhd-tx は loss の後 device を呼ばず，`shutdown` の `Provider::stop` の道で記録だけ |
| cold change | `COMMAND_REJECTED`（`lib.rs:1347, 1938`） | `COMMAND_REJECTED`（`tx.rs:710`，UR-23） |

`parity.rs` の `vh_8_*`（20 本）に held burst を残した `Stop` の場面が無いので，食い違いは見つからなかった．

**D2：`radio.RX_OVERFLOW` の値．**

| 項目 | MockRadio（`lib.rs:745-767`，MR-21） | UHD Module（`rx.rs:556-568`，UR-18） |
|---|---|---|
| `restart_gap_ns` | profile の `overflow_restart_gap_ns`（固定値） | 実際の飛び（`lost`×N を ns に） |
| `lost` | fault ごとの損失 | block の飛び全体 |
| 1 block の前に違う種類の loss（overrun と sequence） | event 2 個，各々自分の分（合計が block の `lost`） | event 2 個，どちらも block の飛び全体（合計が 2 倍） |
| 1 block の前に同じ種類の loss 2 回 | event 2 個，各々自分の分 | event 1 個（同じ種類の report は 1 つにまとめる，`rx.rs:91-95`，UR-19） |
| link が `Full` で block を捨てた（back-pressure） | 捨てた時点で `RX_OVERFLOW`，空白は profile の値（MR-19，`lib.rs:599-615`） | event なし，`stats.link_drops_seen` を数えるだけ（`rx.rs:600`，UR-17） |

spec も決めていない：RM-17 は event の `restart_gap_ns` が固定値か実測かを書かず，重なったときの event の `lost` も書かない．UR-18 は UHD の書き方（実測）を書く．A2（[計画32](../maintenance/32-test-line-sweep.md) の 14 条件）の多くがこの値で，RM-18 と UR-18 の `*Test:*` 行は `restart_gap_ns` と sequence の `cause` を「none named」としている．

**実機で分かること**（UHD 4.10 の source，VERIFIED）：
- `rx_metadata_t` は失った sample 数を持たない（`host/include/uhd/types/metadata.hpp`：`has_time_spec`，`time_spec`，`more_fragments`，`fragment_offset`，`start_of_burst`，`end_of_burst`，`eov_positions*`，`error_code`，`out_of_sequence`）．失った量は次の block の timestamp の飛びからしか分からない．
- X300 系（RFNoC）の overrun の後の再開は，host が受けた restart の要求に対して radio が `get_time_now() + 0.05 s` で `STREAM_MODE_START_CONTINUOUS` を出す（`host/lib/rfnoc/radio_control_impl.cpp:48, 105-117`）．空白は host が気づくまでの遅れに 50 ms を足したもので，一定でない．
- overrun の処理中に来た 2 回目以降の overrun の通知は捨てられ（`host/lib/rfnoc/rfnoc_rx_streamer.cpp:304-310`），利用者には buffer に残ったデータの後に 1 回だけ overflow が報告される（`:328-330`）．

## 2. 決定（owner 2026-10-22，すべて推奨どおり）

1. **`Stop` で捨てた burst は報告しない（記録だけ）．** UHD Module を MockRadio に合わせる．`COMMAND_REJECTED` は RM-10 で「受け付けた Action を Provider が実行できなかった」ときの event で，`Stop` による取り消しは client が頼んだ結果（RM-16）だから．加えて，既定の反応 `mark_artifact`（RS-30）が，正常な停止のたびに開いている capture に印を付けるから．規則は held burst が**外の出来事で取り消される**場合だけを決める：`Stop`・`Provider::stop` は正常な終わり（記録だけ），loss は `DEVICE_LOST` が別に知らせる（記録だけ），cold change は予約した旧 SampleClock の上では実行できなくなった予約の無効化（`COMMAND_REJECTED`，今の両方のまま）．late policy の drop（`TIME_ERROR`），underflow からの回復（`TX_UNDERFLOW`），は専用の event を持つ別の規則で，この規則の外（Sol 指摘 3）．未開始の将来の clock の上の予約は，取り消した理由の規則に従う：cold change がその clock を飛ばしたなら `COMMAND_REJECTED`（両方とも今のまま，`lib.rs:1342-1347`，`tx.rs:242-252`），`Stop`・`Provider::stop`・loss なら記録だけ（再レビュー指摘 1）．
2. **規則は語彙の 1 か所に置き，parity test で固める．**
   - `ezsdr_radio` に取り消しの理由の enum を置く（例：`Cancel { Stop, ProviderStop, Loss, ColdChange }`）．報告するかを返す関数を 1 つだけ持たせる．
   - 両 Provider の bool 引数をこの enum に置き換える．呼び出し側が書くのは理由だけになる．
   - RM-16 に規則を 1 文で書く．
   - `parity.rs` に，held burst を残した `Stop`・device の `Stop`・`Provider::stop`・cold change で，両 Provider の `COMMAND_REJECTED` の数と `rejected` の行の数が一致するテストを足す．
   - reason の文字列は各 Provider の規則 ID を指すので，揃えない．
   - 範囲は held burst の外の出来事による取り消しだけ．受け取った時の拒否（未対応，波形の誤りなど）は常に報告で選ぶ余地が無く，専用の event を持つ drop（決定 1）は別の規則なので，どちらも含めない．pending の update の取り消し（RM-16，MR-20）も含めない（記録の形まで揃える必要は無い）．
3. **D2 もこのメモで扱い，実装は 2 段階に分ける．** 1b-7a が #51（決定 1・2），1b-7b が D2（決定 4・5）．1b-7a が先．
4. **`RX_OVERFLOW` の payload から `restart_gap_ns` を削除し，`{ cause, lost }` にする．**
   - 実機で分かる空白は timestamp の飛びで，それは `lost` と stream の rate から計算できる．sequence では定義上 0 なので，field は何も言わない．
   - 変わる所：RM-11 の payload の表，RM-17・RM-18，RM-22 の `RxOverflowPayload`，RM-24 の hot path の形（17 byte → 9 byte：byte 0 が cause，1〜8 が `lost`），`schemas/radio/rx_overflow_payload.v1.json`（作り直して `schema_freeze` を緑にする．版は上げない：AGENTS.md §6），UR-18，MR-21．
   - envelope の `radio.timing.overflow_restart_gap_ns`（Mock が overrun の空白を作る model の値，bench で測る）は別物で，残す．
5. **1 block の前に loss が重なっても，device か fault の report を受けた後の最初の飛びのある block ごとに `RX_OVERFLOW` は 1 個．**
   - `lost` はその block の飛び全体（block の `lost` と同じ値）．これは「report を伴う区間で失った量」で，原因ごとの内訳ではない．UHD で missed start（100 sample）の後に overrun（50 sample）が同じ block の前に来れば `{ cause: overrun, lost: 150 }`（Sol 指摘 1．MockRadio は start に失敗すると受信を始めないので，この重なりは UHD だけ：MR-11）．
   - `cause` は代表値で，その前の report に overrun が 1 つでもあれば `overrun`，sequence だけなら `sequence`．block の flag（`RESTARTED`，`SEQ_DISCONTINUITY`）は今のまま両方立つ．
   - 飛びの無い block が来たら，report は event を出さずにそこで消える（後の無関係な飛びに付け替えない）．今の UHD は `lost: 0` の event を出しうる（`rx.rs:533-566`）ので，これは変わる（Sol 指摘 4．飛びの無い report が実機で来るかは INFERRED）．report の後に block が来なければ event は出ない．
   - **例外：back-pressure の overrun**（MR-19，RM-17 の最後の文）は今のまま，Mock が block を捨てた時点で，その時に足した損失を報告する．共通の関数は report の後の block の処理だけで，back-pressure は通らない（Sol 指摘 2）．UHD は link の `Full` を数えるだけで event を出さない（UR-17）．Mock は「host が受け取らず device が溢れた」結果を模擬し，実機ではそれが device の本物の overflow として届くので，この差はこのメモの範囲外．
   - 時刻は今と同じ，最初に失った sample．
   - 「その block に付いた report の一覧と block の `lost` → event 0 個か 1 個」の関数を `ezsdr_radio` に置き，両 Provider が呼ぶ（決定 2 と同じ形）．
   - 変えないもの：report の無い飛び（UHD で `SEQ_DISCONTINUITY` だけ，event なし）と `ALIGNMENT_ERROR`．
   - Mock の fault の記録（`ezsdr.radio.mock.faults` の行ごとの `lost`）は fault ごとのまま．

## 3. 消える書き方と変わる文

- 両 Provider の `emit` の bool 引数と，呼び出し箇所ごとの true/false．
- `RxOverflowPayload::restart_gap_ns` と，それを埋める 2 つの計算（Mock の profile の値，UHD の `lost`×N）．
- Mock の fault ごとの `overflows` の event（block ごとの 1 個に）．UHD の report ごとの event（同じ）．
- spec：RM-10（変えない．`COMMAND_REJECTED` の意味はそのまま），RM-11，RM-16（決定 1 の 1 文），RM-17，RM-18，RM-22，RM-24 とそのテスト表（`design/07-radio-model.md:356`），MR-19（back-pressure の event の payload），MR-21，MR-25，MR-37 のテスト表（`design/09-mock-radio.md:322`），UR-18，UR-19（「違う種類の report はそれぞれ event」の RX_OVERFLOW の部分），UR-20（17 byte），UR-23・UR-26（`Stop` の取り消しは記録だけ）．
- schema：`schemas/radio/rx_overflow_payload.v1.json` と `schemas/radio/rx_overflow_hot_payload.v1.json`（長さ 17 の固定）．旧 17 byte と旧 object は拒否する（互換の decoder も版上げも足さない）．

## 4. テストで固める系列

| # | 系列 | 期待 | 段階 |
|---|---|---|---|
| T1 | 送信の held burst を残して `<id>/tx/s0` に `Stop` | 両方とも `COMMAND_REJECTED` なし，`rejected` に burst ごとに 1 行 | 1b-7a |
| T2 | 同じで device の `Stop` | T1 と同じ | 1b-7a |
| T3 | 同じで `Provider::stop` | T1 と同じ | 1b-7a |
| T4 | held burst を残して cold change | 両方とも burst ごとに `COMMAND_REJECTED` 1 個 | 1b-7a |
| T5 | 理由の enum の表 | 4 つの理由それぞれの報告の有無（unit） | 1b-7a |
| T6 | overrun 1 回 | event 1 個，`lost` が block の `lost`，cause `overrun`，hot path 9 byte | 1b-7b |
| T7 | sequence error 1 回 | event 1 個，cause `sequence` | 1b-7b |
| T8 | 1 block の前に overrun と sequence | event 1 個，cause `overrun`，`lost` が block の `lost`，flag は両方 | 1b-7b |
| T9 | 1 block の前に overrun 2 回 | event 1 個，`lost` が block の `lost`（UHD は今のまま固定，Mock は 2 個から 1 個に） | 1b-7b |
| T10 | report の無い飛び（UHD） | `SEQ_DISCONTINUITY`，event なし（今のまま） | 1b-7b |
| T11 | `RxOverflowPayload` の hot path | 9 byte の round trip，別の長さと cause の byte の拒否 | 1b-7b |
| T12 | 等式 | event の `lost` の合計 = report のある飛びを持つ block の `lost` の合計（back-pressure なし，event の ring の drop なしの条件で） | 1b-7b |
| T13 | loss で held burst が消える | 両方とも `COMMAND_REJECTED` なし，`rejected` に記録 | 1b-7a |
| T14 | 未開始の将来の clock の上の burst の取り消し | cold change がその clock を飛ばしたなら両方とも `COMMAND_REJECTED`（今のまま），`Stop` なら両方とも記録だけ | 1b-7a |
| T15 | RX の `Stop` の後と TX の `Stop` の後 | RX の `Stop` は TX の予約を消さない．TX の `Stop` の後の新しい burst は送られる | 1b-7a |
| T16 | 取り消した burst | 送られず，予約も残らない（数の一致だけでなく，送信の記録と held の空で見る） | 1b-7a |
| T17 | UHD で missed start と overrun が 1 block の前 | event 1 個，cause `overrun`，`lost` が両方の飛び | 1b-7b |
| T18 | report の後に飛びの無い block | event なし，report はそこで消え，後の飛びに付かない | 1b-7b |
| T19 | report の後に block が来ない（stop） | event なし | 1b-7b |
| T20 | back-pressure：fault の後の block が `Full`，`Full` が続く，`Full` の後に block なし | Mock は捨てた時点ごとにその時の損失を報告し，2 重に数えない | 1b-7b |

**両段階とも risky** とし，テスト計画を先に書く（AGENTS.md §9）．1b-7a は Mock と UHD の報告の境を変え，`COMMAND_REJECTED` の数は Policy の `stop`／`abort` を通じて Run の終わり方も変えうる（RS-26，Sol 指摘 4）．1b-7b は fake（Mock）と実機の境（Mock が実機より細かく報告していたのを揃える）と hot path の形を変える．

## 5. 採らなかった案

- 決定 1：**報告する（UHD に揃える）**：`COMMAND_REJECTED` の意味に合わず，正常な停止で capture に印が付く．**新しい event kind（`BURST_CANCELLED` など）**：受け取る利用者がいない．
- 決定 2：**spec の 1 文と parity test だけ（bool を残す）**：散文から 2 回書く形が残り，テストに無い場面で次の理由が足されるとまた割れる．**held burst の扱い全体を共有のコードに**：UHD の held burst は device の handle と report の待ちを持ち，持ち方が違う．共有できるのは報告の判断だけ．**すべての拒否を 1 か所に**：受け取った時の拒否には選ぶ余地が無い．
- 決定 3：**メモを分けて #51 を先に閉じる**：「報告の規則を語彙に置く」理由を 2 回書くことになる．段階を分ければ速さは同じ．**D2 を実機の後に**：Mock が実機と違う値を出したまま parity を回すことになる（§59）．
- 決定 4：**残して両方とも実測の飛び**：何も言わない field が hot path に残り，また割れる余地が残る．**残して両方とも profile の固定値**：実機では空白が一定でなく，嘘を報告する．
- 決定 5：**report ごとに 1 個，2 個目以降の `lost` は 0**：意味の無い event が出る．**cause ごとに 1 個**：違う cause のとき `lost` をどちらに入れるかの決めごとが増える．

## 6. 未決

- 1b-7b のテスト計画で：飛びの無い report が実機で来るか（INFERRED）．来ないなら T18 は fake でだけ固める．
- 範囲外として記録：back-pressure の扱いは Mock（event と空白を模擬）と UHD（数えるだけ）で違う．Mock は host が受け取らない結果を模擬し，実機ではそれが device の overflow として届くので，同じ規則の 2 つの写しではない．

## 7. レビュー

Sol 6.1（2026-10-22）：修正して採用．major 3，minor 3．すべて採用した．`restart_gap_ns` の削除，語彙の小さな共通の関数と parity test，Kernel を変えないことは支持された．UHD 4.10 の §1 の VERIFIED も source で確かめられた．
- major 1：決定 5 の `lost` は「report を伴う区間の損失」で，`cause` は代表値と明記（missed start と overrun の重なり）．
- major 2：back-pressure の overrun は決定 5 と T12 の例外として残す．
- major 3（Sol の 4）：1b-7a も risky．loss，未開始の clock，RX/TX の `Stop` の後，取り消した burst が送られないこと，飛びの無い report と block の来ない report を系列に足した．
- minor 1（Sol の 3）：決定 1 の規則を held burst の外の出来事による取り消しに限り，専用の event を持つ drop を外した．
- minor 2（Sol の 5）：UR-19，UR-20，MR-19，MR-37 と RM-24 のテスト表，hot payload の schema を §3 に足した．
- minor 3（Sol の 6）：§1 の表を違う種類と同じ種類に分け，`vh_8_` の数を 20 に直した．

再レビュー（同日）：修正して採用．major なし，minor 1：未開始の将来の clock の上の予約は，取り消した理由の規則に従う（cold change なら報告，`Stop` なら記録だけ）．採用して決定 1 と T14 に入れた．T1〜T20 はテスト計画の土台として十分とされた．

## 8. 実装

- 1b-7a：完了（2026-10-23）．決定 1・2 は `ezsdr_radio::CancelReason` と `cancellation_reports`．UHD の loss は shutdown を通るので `ProviderStop` を渡す（どちらも記録だけ）．記録は [reviews/1b7a-opus.md](reviews/1b7a-opus.md)．
- 1b-7b：完了（2026-10-23）．決定 4・5 は `ezsdr_radio::payloads::rx_overflow_payload`．`lost` は block の飛びのうち先の event が報告していない分：MockRadio の back-pressure の後の fault は，T20 のとおり報告済みの損失を数えない（実装は一度 2 重に数え，Opus と Sol が拾った）．記録は [reviews/1b7b-opus.md](reviews/1b7b-opus.md)．
