STATUS: plan

# R2 テスト計画：UHD Module の stream ごとの段階を遷移の関数だけが変える

2026-10-22、実装段階 1b-6b（`plan/phase8/35-uhd-stream-state.md` §2 R2）の risky step 2。
対象は `crates/ezsdr-radio-uhd/src/provider/core.rs` の `StreamState` の `idle`・`active`・`delivered` を
private な `Phase` にし、`Streams` の遷移の関数だけが変えるようにすることだけ。
`Claims`（R1）、device の記録、`completed`、`clocks`、loss、`needed_before` の判定は変えない。

**挙動は変えない。** HEAD `2af9afd` のコードを正とし、メモ R2 の表と HEAD が合わない所は §5 に書く。
この文書は計画の修正版。前段階でコードの一部が計画より先に書かれた（順序が逆）。この修正の後に P6/P7 の残り・N8・spec 18・mutation 行を書き、§7 のゲートを実行した。
Sol の計画レビュー（`sol-plan.log` の最後）への修正は §9 に一覧する。

## 1. 置く型と関数

```rust
pub(crate) type Segment = (i64, Config, Option<u64>);

pub(crate) enum Phase {
    Idle,                                                              // 始める segment が無い
    Pending,                                                           // 始める segment はあるが、まだ始めていない
    Active { segment: Option<Segment>, delivered: Option<(i64, Config, i64)> }, // delivered は受信だけ
}
// Default は Pending: `StreamState::default` は今の `idle = false` を保たなければならない
// (Core::new の `Streams::with_streams` が start 前に使う)。

pub(crate) struct StreamState {
    // ... configs / set / generation / configured / completed / claims / clocks はそのまま
    phase: Phase,   // private。`idle`・`active`・`delivered` は消える
    // ...
}

impl Streams {
    // 遷移（これだけが phase を書く）
    pub fn starting(&mut self, stream: StreamKey, held: bool);                     // Control::new
    pub fn search(&mut self, stream: StreamKey, found: bool);                      // Rx::begin
    pub fn started(&mut self, stream: StreamKey, segment: Option<Segment>, origin: i64, config: Config); // Rx::new / Rx::begin
    pub fn delivered(&mut self, stream: StreamKey, origin: i64, config: Config, k: i64); // Rx::samples
    pub fn finished(&mut self, stream: StreamKey) -> Option<Segment>;              // Rx::finish / Tx::do_switch
    pub fn plan_read(&mut self, stream: StreamKey, clock: bool, switch: bool, segment: Option<Segment>); // Tx::plan_section

    // 読むだけ
    pub fn phase(&self, stream: StreamKey) -> Phase;
    pub fn is_idle(&self, stream: StreamKey) -> bool;
    pub fn active(&self, stream: StreamKey) -> Option<Segment>;
    pub fn first_undelivered(&self, stream: StreamKey) -> Option<(i64, Config, i64)>;
}
```

`Phase` は `Clone, Copy, PartialEq, Eq, Debug` を derive する（`Config` が `Copy` なので `Segment` も `Copy`）。
`phase` はテストの観測用で、production は `is_idle`・`active`・`first_undelivered` だけを読む。
`delivered` の tuple を残す理由は §5 の食い違い 3。

### 1.1 遷移の表（テストが固定する内容）

| 関数 | 呼ぶ所 | 前 → 後 | 前が違うとき |
|---|---|---|---|
| `starting(stream, held)` | `Control::new`（RX は T0 の stream、TX は clock の有無） | `held` なら `Pending`、でなければ `Idle` | —（初期化） |
| `search(stream, found)` | `Rx::begin`（holder が `Free` と確かめた後） | `found` なら `Pending`、でなければ `Idle` | 前の `Active` は消える（production では `begin` は stream が無いときだけ走るので到達不能。N4 が固定） |
| `started(stream, segment, origin, config)` | `Rx::new`（T0 の stream があるときだけ）と `Rx::begin` の成功 | `Active { segment, delivered: Some((origin, config, 0)) }` | — |
| `delivered(stream, origin, config, k)` | `Rx::samples`（ブロックを渡した） | `Active` の `delivered = Some((origin, config, k))` | `Active` でなければ何もしない（N5 が Pending 中の書きを固定） |
| `finished(stream)` | `Rx::finish`、`Tx::do_switch` | `Pending`。旧 `Active` の `segment` を返す | `Active` でなければ `None` を返し `Pending` |
| `plan_read(stream, clock, switch, segment)` | `Tx::plan_section` | `clock` なら `Active { segment, delivered: None }`、`switch` だけなら `Pending`、どちらも無ければ `Idle` | — |

読む側の置き換え：

| 今の読み | 置き換え |
|---|---|
| `control.rs:836` の `streams.stream(stream).idle`（from-none の判断） | `streams.is_idle(stream)` |
| `core.rs:310` の `claim_release` の `!state.idle \|\| state.active.is_some()` | `!self.is_idle(stream) \|\| self.active(stream).is_some()` |
| `core.rs:1058` の `release_unneeded` の同じ判定 | 同上 |
| `core.rs:397` の `book` の `self.stream(stream).delivered` | `self.first_undelivered(stream)`（tuple を返すので `cuts` の照合はそのまま） |
| `tx.rs:720-721` の `do_switch` の `active` 完了 | `streams.finished(stream)` の戻り値を `complete` へ |
| `rx.rs:371-373` の `finish` の `active` 完了 | 同上 |

`needed_before` と `completed` は今のまま（全 planned segment と `completed` を照合し、phase を読まない）。

### 1.2 HEAD の印との対応

| HEAD（到達できる組み合わせ） | Phase |
|---|---|
| `idle = true`、`active = None` | `Idle` |
| `idle = false`、`active = None`（TX の init、RX の探索後・拒否後・`finish` 後） | `Pending` |
| `active = Some(seg)`、`delivered = Some(k)`（RX） | `Active { segment: Some(seg), delivered: Some(k) }` |
| `active = Some(seg)`、`delivered = None`（TX） | `Active { segment: Some(seg), delivered: None }` |
| `active = None`、`delivered = Some(k)`（RX init で T0 の segment が plan に無い） | `Active { segment: None, delivered: Some(k) }` |

HEAD では test が `idle = true` と `active = Some` を同時に作れたが、その組み合わせは production では到達できない
（`begin` は `next` が無ければ `active` を立てず、`plan_section` は `idle` と `active` を同じ節で決める）。
enum ではこの組を表せなくする。§4 の `ur_25_a_claim_release_needs_...` は 2 つの前提を別々に確かめる形に直す。

## 2. 系列 P1–P11

「今あるテスト」は HEAD `2af9afd` の名前（行番号は目安）。新しいテストの詳細は §3。

| 系列 | 今あるテスト（何を assert しているか） | 新しいテスト／変更 | 観測 | 落とす誤実装 |
|---|---|---|---|---|
| P1 受信の初期化：T0 の stream を持つ／持たない | `ur_26_a_stream_s_delivered_is_cleared_alone`（`rx.rs:995`）: stream 1 を T0 付きの `Rx::new` で作り、`finish` が stream 1 の floor だけを消す。T0 の Active 自体はまだ読んでいない | N1（`starting`）。既存のこのテストに、`Rx::new` 後の `phase == Active { Some((0, cfg(1), None)), Some((0, cfg(1), 0)) }` と、`finish` 後の `Pending`・floor `None` を足す | `phase`、`first_undelivered`、`is_idle` | `starting(held=true)` が `Idle`（T0 の stream が from-none に取られ、最初のブロックの floor が消える）、`starting(held=false)` が `Pending`（from-none が走らず、`release_unneeded` が閉じない）、`Rx::new` が T0 の `segment` を落とす |
| P1b 受信の初期化で T0 の segment が plan に無い | 直接見るテストは無い。`ur_15_a_loss_published_by_an_owner_stops_the_next_stream_s_start` は start の途中の loss の筋だが phase を読まない | N2（`Streams` unit）、N3（T0 の line を作り、T0 前の loss を実際に book して segment を消してから `Rx::new`・`finish`。thread 無し） | `phase`（`Active { None, Some((t0, cfg, 0)) }`）、`first_undelivered`（tuple は残る）、`completed`、loss の `End` の instant、plan が空のまま | `started(None)` が `Pending`／`Idle`（T0 の stream が段階に無い）、`first_undelivered` が `segment: None` を理由に tuple を落とす（N2・N3）、`finished` が `None` の segment を `completed` に載せる、`Rx::new` が `segment: None` を `Active` にしない |
| P2 受信の `begin`：cursor 以降に segment がある／無い／holder が `Free` でない | `ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing`（`control.rs:2702`）: holder が from-none の間 `rx.begin` が false、cursor と idle と device call が不変。TX 側は `ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call`（`tx.rs:2220`） | N4（`search`）。既存 2 本の `idle` 読みを `is_idle` に（§4） | `phase`、`is_idle`、`active`、`claim_release`、既存の cursor/device call | `search(found=true)` が `Active`（まだ configure も start もしていない segment を「処理中」にする）、`search(found=false)` が `Pending`（from-none が idle を取れない）、holder の early return が phase を動かす（P382） |
| P3 受信が segment を始め、ブロックを渡し、終える | `ur_26_a_receive_start_queued_by_the_stop_ends_its_owner_without_detaching`（`rx.rs:944`）: `begin` が true、start が発行され、ブロック無し、origin で untimed stop。`ur_17_an_overlap_trimmed_to_nothing_is_dropped`（`rx.rs:1018`）: ブロックの重なり。`ur_29_a_loss_ends_the_stream_at_its_instant`（`rx.rs:1158`）: floor の終端 | N5（`search → started → delivered → finished`）。既存の stop テストに、exit 後の `phase == Pending`（`is_idle == false`）と `first_undelivered == None` を足す | `phase`、`first_undelivered`、`planned().cut`、device call、clocks | `started` が `delivered: Some(0)` を置かない（最初のブロックの floor が無い）、`delivered` が `Pending`/`Idle` にも書く（終えた segment の floor が次へ漏れる）、`finished` が `Active` を残す（次の `search` が active を見る、`do_switch` が二重に complete する） |
| P4 受信の configure が拒否される／loss | `ur_25_a_refused_segment_is_booked_by_uhd_rx`（`rx.rs:1436`）: 拒否が line に載り、command 2 の `Start` で再開し、その origin で start する（rate 100）。`ur_29_a_loss_after_a_configure_starts_no_receive_segment`（`rx.rs:935` 付近）: configure 中の loss で戻る | 両方に phase の assert を足す：拒否直後・loss の後は `Pending`、`active == None`、floor `None`。再開後は `active == Some((ms(65), _, Some(2)))`（command 2 の `Start`） | `phase`、`active`、`first_undelivered`、line の `refused`、device call | 拒否後に `Active`（refused segment を処理中にする）、拒否後に `Idle`（from-none が次の turn の前に取る）、再開の identity を拒否された command 1 にする |
| P5 送信の `plan_section`：clock も switch も無い／clock がある／clock が無く switch がある | `ur_25_uhd_tx_reads_the_plan_again_when_it_changes`（`tx.rs:1477`）: `read` が switch と新しい clock を採る | N6（`Tx::read` の 4 状態。clock があり plan に segment が無い枝を含む）。既存に、最初の `read` 後 `Active { Some((0, cfg(200), None)) }`、switch を持つ `read` 後もなお `Active` を足す | `phase`、`is_idle`、`active`、`first_undelivered == None` | `plan_read` が `switch` と `clock` を取り違える（switch だけの回が `Active`、clock の回が `Pending`）、switch を無視する（次の segment へ渡らない）、TX に `delivered: Some(0)` を置く |
| P6 送信の `do_switch` が拒否される：次の segment が無い／有る | `ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch`（`tx.rs:1532`）: 拒否された switch は clock を採らず `tx_burst` を拒否し、受理された switch は次の clock を採る。3 個の変更を予約し、3 回目の switch は 3 個目の segment を採る（次は無い状態ではない） | 既存に、各 `do_switch` 後の phase と `completed` を足す：拒否 → `Pending`、受理 → 次の `Active`、最後 → 3 個目の `Active`、switch 元が `completed` に載る。**別に** `ur_25_a_refused_last_transmit_change_ends_idle` を足す：後続の無い拒否で旧 segment が `completed`、clock 無し、最後が `Idle` | `completed`、`phase`、`active`、`tx.clock`、device call | 旧 `Active` を complete しない（`needed_before` が離した segment を必要とし続ける）、拒否後に `Idle`（次があるのに from-none が取る）、後続の無い拒否で `Pending`（`is_idle` が false のまま） |
| P7 送信の空への switch、長さ 0 の segment の skip、拒否済みの segment の skip | `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again`（`tx.rs:1150`）: B → 空 → C、長さ 0 の中間 segment を飛ばし、owner が各 set を開き直す。`ur_25_a_refused_transmit_clock_is_not_adopted`（`tx.rs:1289`）: 拒否された clock を採らない | 既存の from-none テストに、`read` 後 `Pending`、B の後の `Active`、skip の後の `Pending`（次の switch 待ち）、C の後の `Active`、B が `completed` に載ることを足す | `phase`、`completed`、`tx.clock`、device call | 長さ 0 の segment を `Active` にする（device call も clock も無い segment を処理中にする）、拒否済み segment を `Active` にする、skip の後に `Idle` |
| P8 `release_unneeded` と from-none の判断 | `ur_25_a_claim_release_needs_a_free_idle_stream_with_no_active_segment`（`core.rs:1842`）: `Free`・idle・active 無しだけを取る。`ur_34_a_release_closes_an_idle_streamer_needed_by_a_move`（`core.rs:1475`）、`ur_34_a_release_keeps_an_idle_streamer_with_no_peer_need`（`core.rs:1487`）、`ur_34_a_release_skips_a_stream_held_busy_by_from_none`（`core.rs:1457`）。from-none は `ur_25_an_enable_behind_an_owner_s_open_is_a_cold_change`（`control.rs:2818`） | 既存の claim_release テストを phase で書き直し、`Pending` と `Active` を別々に拒む。他は setup の `idle` 書きを `search` に（§4）。新しいテストは足さない | `claim_release` の戻り値、`is_idle`、device call、`set` | `is_idle` が `Pending` を true にする（from-none が owner の見つけた segment を奪う、release が始まる前の streamer を閉じる）、`claim_release` が `Active` を取る（処理中の streamer を閉じる） |
| P9 `Streams::book` の `End`（loss、`Provider::stop`） | `ur_26_an_end_takes_delivered_from_each_segment_s_own_stream`（`core.rs:1780`）: stream ごとの floor、1 line 1 End。`ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts`（`rx.rs:1252`）: 走っている segment の floor だけ。`ur_26_an_orderly_stop_cuts_at_its_instant`（`control.rs:2009`）: floor 無しは stop の instant、6000 は 6000。`ur_29_a_loss_ends_the_stream_at_its_instant`（`rx.rs:1158`）。`ur_26_two_receive_streams_end_at_their_own_stops`（`fake.rs:7332`） | 最初の 2 本と orderly stop の setup を `started` + `delivered` に（§4）。N3 が `segment: None` でも tuple を残す場合を持つ。N7 が Abort の床を持つ | `planned().cut`、`first_undelivered`、`Item.delivered` | `first_undelivered` が `Active` でない stream の floor を返す（P445）、segment の照合を落とす（J54）、floor をブロックの先頭にする（P258） |
| P10 有限の move の鎖、同時の swap、source の拒否の前の destination の待ち | `ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain`（`fake.rs:6130`）、`ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock`、`ur_34_a_waited_open_waits_for_same_move_source_configuration`（`core.rs`）、`ur_34_a_move_destination_waits_for_source_busy_after_channel_release`（`core.rs`）、`ur_34_a_waited_open_waits_for_a_finite_unprocessed_move_segment`（`core.rs`） | 変更なし（`needed_before` と `completed` は phase を読まない）。緑のままであることを gate で確かめる | 既存の device call の順、`completed`、`needed_before`、clocks | `finished` が `complete` を呼ばない／余分な segment を complete する、`needed_before` が phase を読み始める（判定が変わる） |
| P11 P472 の系列 | `ur_26_two_receive_streams_end_at_their_own_stops`（`fake.rs:7332`）: thread 版。stream 1 の Stop の後、Abort が stream 0 を自分の delivered で切る | N7（thread 無しの `Streams` unit。Abort の End を stream 0 の delivered より後に置く）。P472 を N7 に当て直す（§6）。fake のテストは残す | `first_undelivered`、`planned().cut`、`completed` | `finished` が全 RX stream の phase を消す（P470/P472）、Abort の End が stream 0 の floor を読まない、Orderly の instant で切る |

## 3. 新しいテスト

すべて thread を使わない。sleep を判定に使わない。既存の thread テスト（P2・P3・P4・P6・P7）は
既存の `holder.arm()` / `probe` / `recv_timeout` の形のままで、phase の assert を足すだけ。

### N1 `ur_25_a_stream_starts_idle_or_waiting`

- 目的：P1 と `StreamState::default` の `Pending`（start 前の `Core::new`）。
- 準備：`Streams::with_streams(1, 1)`、`k = stream_key(Dir::Rx, 0)`、`t = stream_key(Dir::Tx, 0)`。
- 手順と assert：
  1. 既定：`phase(k) == Pending`、`phase(t) == Pending`、`!is_idle`、`active == None`。`claim_release(k)` は `None`（start 前の stream を release が閉じない）。
  2. `starting(k, false)`：`phase == Idle`。`claim_release(k)` が `Some`（`give_back` して `Free`）。
  3. `starting(k, true)`：`phase == Pending`、`!is_idle`、`active == None`。
  4. `starting(t, true)`：`Pending`（HEAD の TX init。§5 の食い違い 1）。`starting(t, false)` で `Idle`。
- 観測：`phase`・`is_idle`・`active`・`claim_release`。
- 落とす誤実装：既定が `Idle`（1 の `claim_release` が `Some` になり、production では start 前の streamer を閉じ得る）、`starting(true)` が `Idle`／`Active`、`starting(false)` が `Pending`。

### N2 `ur_26_a_receive_start_without_a_segment_stays_active`

- 目的：P1b。T0 の segment が plan に無いときの `Active { None, Some((0, cfg(1), 0)) }` と、floor の tuple を残すこと。
- 準備：`Streams::with_streams(1, 1)`、`k = stream_key(Dir::Rx, 0)`。
- 手順と assert：
  1. `started(k, None, 0, cfg(1))`：`phase == Active { segment: None, delivered: Some((0, cfg(1), 0)) }`、`!is_idle(k)`。
  2. `first_undelivered(k) == Some((0, cfg(1), 0))`（HEAD の tuple を残す）。
  3. `delivered(k, 0, cfg(1), 500)`：`first_undelivered(k) == Some((0, cfg(1), 500))`。
  4. `finished(k) == None`、`phase == Pending`、`first_undelivered == None`、`stream(k).completed.is_empty()`。
- 観測：`phase`・`first_undelivered`・`completed`。
- 落とす誤実装：`started(None)` を `Pending`/`Idle` にする、`first_undelivered` が `segment: None` を理由に tuple を落とす（2 が落ちる）、`finished` が `None` を `completed` に載せる。

### N3 `ur_26_a_receive_start_of_no_planned_segment`

- 目的：P1b の実 path。T0 の line を作り、T0 前の loss を実際に book して T0 の segment を plan から消してから `Rx::new`・`finish` を通す。
- 準備：`rig_with_links(vec![link()])`。stream 0 の line を `origin = t0 (10 ms)`、`Config` は 1 channel・1 MS/s で入れ、`streams.end(&core, 5 ms, false, false)` を呼ぶ。`planned(key)` が空になったことを assert。`SampleClock` を宣言し、`Rx::new(core, 0, cmds, Some((handle, Channels::first(1), t0)), None)`。`cmds` の送信側は `mem::forget`（既存テストと同じ）。
- 手順と assert：`phase == Active { None, Some((t0, config, 0)) }`、`first_undelivered == Some((t0, config, 0))`。`rx.finish()` の後、`phase == Pending`、`first_undelivered == None`、`completed.is_empty()`、`planned(key)` は空のまま、line の `End` は 1 個で `e == 5 ms`（loss の End と cut が変わらない）。
- 観測：`phase`・`first_undelivered`・`completed`・`planned`・`Kind::End` の `e`。
- 落とす誤実装：`Rx::new` が `segment: None` を `Active` にしない（`Pending` のまま）、`first_undelivered` が tuple を落とす、`finish` が loss の `End` を書き換える／segment を捏造する。

### N4 `ur_25_a_search_leaves_the_stream_pending_or_idle`

- 目的：P2。探索の結果と `claim_release` の前提。
- 準備：`Streams::with_streams(1, 1)`、`k = stream_key(Dir::Rx, 0)`。
- 手順と assert：
  1. `search(k, true)`：`Pending`、`!is_idle`、`active == None`。`claim_release(k)` は `None`。
  2. `search(k, false)`：`Idle`、`claim_release(k)` が `Some`（`give_back`）。
  3. `started(k, Some((0, cfg(1), None)), 0, cfg(1))` の後に `search(k, false)`：`Idle`、`active == None`（production では到達不能な組だが、phase が 1 値であることを固定する。§5 食い違い 5）。
- 観測：`phase`・`is_idle`・`active`・`claim_release`。
- 落とす誤実装：`search(true)` が `Active`、`search(false)` が `Pending`、`search` が前の `Active` を消さない（3 が落ちる）。

### N5 `ur_17_a_receive_segment_moves_from_pending_to_active_to_pending`

- 目的：P3。`search → started → delivered → finished` の 1 本の鎖。
- 準備：`Streams::with_streams(1, 1)`、`k`、`seg = (0, cfg(1), None)`。
- 手順と assert：
  1. `search(k, true)`：`Pending`。
  2. `delivered(k, 0, cfg(1), 500)` を `Pending` 中に呼ぶ：`phase == Pending`、`first_undelivered == None`（書き込まない）。
  3. `started(k, Some(seg), 0, cfg(1))`：`Active { Some(seg), Some((0, cfg, 0)) }`。
  4. `delivered(k, 0, cfg(1), 3_000)`：`first_undelivered == Some((0, cfg, 3_000))`。
  5. `finished(k) == Some(seg)`、`Pending`、floor `None`、`completed.is_empty()`。さらに `delivered(...)` を呼んでも `Pending` のまま（終えた segment の floor が戻らない）。
  6. `complete(k, seg)`：`completed == [seg]`（`complete` は遷移の外）。
- 観測：`phase`・`first_undelivered`・`completed`。
- 落とす誤実装：`started` が `delivered: Some(0)` を置かない、`delivered` が Pending/Idle にも書く（2・5）、`finished` が `Active` を残す／`complete` を自分で呼ぶ。

### N6 `ur_25_a_transmit_read_sets_idle_pending_or_active`

- 目的：P5。`plan_section` の 3 状態と、TX に floor が無いこと。clock があり plan に segment が無い枝も見る（Sol の計画レビュー）。
- 準備：`rig()`（`Streams::default()`、line 無し）、`k = stream_key(Dir::Tx, 0)`。
- 手順と assert：
  1. `Tx::new(core, 0, cmds, None, Channels::first(0))`、`read(None)`：`phase == Idle`。
  2. `clock = core.register(Dir::Tx, 200, 0)`、`Tx::new(..., Some(clock), Channels::first(1))`、`read(None)`（line 無し）：`Active { segment: None, delivered: None }`、`first_undelivered == None`。
  3. line を入れ（`origin 0`、`cfg(200)`、`lead` は既存テストと同じ）、clock 無しの `Tx` で `read(None)`：`switch.is_some()`、`Pending`。
  4. clock 付きの `Tx` で `read(None)`：`Active { Some((0, cfg(200), None)), None }`、`!is_idle`、`first_undelivered == None`。
- 観測：`phase`・`is_idle`・`active`・`first_undelivered`・`tx.switch`。
- 落とす誤実装：`plan_read` が `clock` と `switch` を取り違える（3 と 4 が入れ替わる）、switch を無視する（3 が `Idle`）、TX に `delivered: Some(0)` を置く（4 の floor が `Some`）、clock があっても segment が無ければ `Pending`（2 が落ちる）。

### N7 `ur_26_a_finished_stream_clears_only_its_own_floor`

- 目的：P11（P472 の系列）。thread 無しで、stream 1 の `finish` が stream 0 の floor を残すこと。Abort の End を stream 0 の delivered より後に置く（Sol の計画レビュー）。
- 準備：`two_by_two()`、`rx0`・`rx1`、`seg = (0, cfg(1), None)`。
- 手順と assert：
  1. `started(rx0, Some(seg), 0, cfg(1))`、`started(rx1, Some(seg), 0, cfg(1))`、`delivered(rx0, 3_000)`、`delivered(rx1, 500)`。
  2. `finished(rx1) == Some(seg)`。`first_undelivered(rx0) == Some((0, cfg(1), 3_000))`（残る）、`first_undelivered(rx1) == None`（消える）。
  3. `streams.end(&core, 800_000, true, false)`（Abort、root tick 800 000 = sample 4 000。stream 0 の 3 000 より後）。
  4. `planned(rx0)[0].segment.cut == Some(3_000)`（stream 0 は instant 4 000 ではなく自分の floor）、`planned(rx1)[0].segment.cut == Some(4_000)`（stream 1 は走っていないので instant）。
- 観測：`first_undelivered`・`planned().cut`。
- 落とす誤実装：`finished` が全 RX stream の phase を消す（2 で rx0 が `None`、4 で 3 000 でなく 4 000）、`book` が全 stream の floor を読む（P445）、`end` の End が stream ごとの line でない、Abort が instant で切る。

### N8 `ur_25_a_refused_last_transmit_change_ends_idle`（P6 の別系列）

- 目的：P6 の「後続の無い configure の拒否」。既存の burst テストは 3 個目の segment を採るので、この系列を別に持つ。
- 準備：`rig()`、`key = stream_key(Dir::Tx, 0)`、`c0`・`c1` を `core.register`、line の plan を `[origin 0, cut c0.at_or_after(e), cfg(200), by None]` と `[origin e, cut None, cfg(100), by Some(1)]` に直接置き、`clocks = [(c0, false), (c1, false)]`。
- 手順と assert：`Tx::new(..., Some(c0), ...)`、`read(None)` で `switch` ができる。`switch.to.refused = true` にして `do_switch()`：`tx.clock == None`、`tx.switch == None`、`phase == Idle`、`active == None`、`completed` に `(0, cfg(200), None)`、device call 無し。
- 観測：`phase`・`active`・`completed`・`tx.clock`・device call。
- 落とす誤実装：旧 `Active` を complete しない、拒否後に `Idle` でなく `Pending` のまま、拒否なのに device call をする。

## 4. 印を直接読み書きしている今のテストと置き換え

HEAD `2af9afd` の `grep -rnE "\.(idle|active|delivered)\b" crates/ezsdr-radio-uhd` で、
production 19 行・test 54 行（`Item.delivered` と `manifest.events.delivered` を除く）。
production の 19 行は §1 の遷移・読みに置き換え、直接の読み書きは 0 にする。test 側の全位置と置き換え：

### `core.rs`

| 行 | テスト | 今 | 置き換え |
|---|---|---|---|
| 1462 | `ur_34_a_release_skips_a_stream_held_busy_by_from_none` | `state.idle = true` | `streams.search(stream, false)` |
| 1477 | `ur_34_a_release_closes_an_idle_streamer_needed_by_a_move` | `stream_mut(from).idle = true` | `streams.search(from, false)` |
| 1492 | `ur_34_a_release_keeps_an_idle_streamer_with_no_peer_need` | `state.idle = true` | `streams.search(stream, false)` |
| 1788,1789,1799,1800 | `ur_26_an_end_takes_delivered_from_each_segment_s_own_stream` | `delivered = Some((0, cfg(1), n))` | `started(k, Some((0, cfg(1), None)), 0, cfg(1))` の後 `delivered(k, 0, cfg(1), n)` |
| 1850,1862,1868 | `ur_25_a_claim_release_needs_a_free_idle_stream_with_no_active_segment` | `idle = true` | `search(k, false)`（Idle） |
| 1859 | 同上 | `idle = false` | `search(k, true)`（Pending） |
| 1863,1865 | 同上 | `active = Some(...)` / `active = None` | `started(k, Some((0, cfg(1), None)), 0, cfg(1))` / `finished(k)`（`finished` は `Some` を返すので `let _`） |
| 1898 | `ur_25_a_claim_from_none_admits_a_read_in_flight` | `idle = true` | `search(k, false)` |
| 1924 | `ur_25_a_begin_read_needs_a_free_stream_and_no_read` | `idle = true` | `search(k, false)` |
| 1954 | `ur_25_a_begin_shutdown_read_ignores_the_holder` | `idle = true` | `search(k, false)` |
| 2030 | `ur_25_move_source_busy_reads_the_owner_of_the_move` | `idle = true` | `search(from, false)` |
| 2040 | `ur_34_a_release_gives_back_a_claim_when_loss_arrives_after_the_claim` | `idle = true` | `search(from, false)` |

注意：`ur_25_a_claim_release_needs_...` の HEAD は「idle かつ active あり」という production で到達しない組を作る。
置き換えでは `Pending`（idle でない、active なし）と `Active`（active あり）を別々に確かめ、その組は作らない。

### `rx.rs`

| 行 | テスト | 今 | 置き換え |
|---|---|---|---|
| 1000 | `ur_26_a_stream_s_delivered_is_cleared_alone` | stream 0 の `delivered = Some((0, cfg, 700))` | `started(rx0, Some((0, config, None)), 0, config)` + `delivered(rx0, 0, config, 700)` |
| 1015,1016 | 同上 | `delivered` の読み | `first_undelivered(rx0) == Some((0, config, 700))`、`first_undelivered(rx1) == None` |

### `tx.rs`

| 行 | テスト／helper | 今 | 置き換え |
|---|---|---|---|
| 1107 | `tx_channels_from_none` | `stream_mut(...).idle = true` | `search(stream_key(Dir::Tx, 0), false)` |
| 2199 | `ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing` | `let idle = \|\| ... .idle` | `let idle = \|\| lock(...).is_idle(...)` |
| 2238 | `ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call` | `... .idle` | `is_idle(...)` |

### `control.rs`

| 行 | テスト／helper | 今 | 置き換え |
|---|---|---|---|
| 1213,1216 | `ur_13_a_failed_open_from_no_stream_is_opened_again_by_the_owner` | `idle = true` | `search(booked_stream(Dir::Rx), false)` |
| 1223 | `enable_from_none` | `idle = true` | `search(booked_stream(Dir::Rx), false)` |
| 1336,1395 | `ur_34_a_from_none_join_yields_to_an_earlier_unprocessed_segment` | `idle = true` | `search(a/b, false)` |
| 1439 | `ur_34_a_control_books_the_next_action_while_an_owner_close_is_gated` | `idle = true` | `search(stream_key(Dir::Rx, 1), false)` |
| 1691 | `ur_24_a_command_waiting_for_a_channel_is_given_before_a_later_one` | `idle = false` | `search(k, true)` |
| 1698 | 同上 | `idle = true` | `search(k, false)` |
| 1731 | `ur_24_a_command_waiting_for_a_channel_keeps_a_later_booking_behind_it` | `idle = true` | `search(k, false)` |
| 1823 | `channels_from_none` | `idle = true` | `search(booked_stream(Dir::Rx), false)` |
| 2020 | `ur_26_an_orderly_stop_cuts_at_its_instant` | `delivered = delivered.map(...)` | `delivered` が `Some(k)` なら `started(k, Some((0, config, None)), 0, config)` + `delivered(k, 0, config, k)`。`None` なら何もしない |
| 2674 | `rx_enable_from_none` | `idle = true` | `search(booked_stream(Dir::Rx), false)` |
| 2726 | `ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing` | `... .idle` | `is_idle(...)` |
| 2833,2838 | `ur_25_an_enable_behind_an_owner_s_open_is_a_cold_change` | `... .idle` | `is_idle(...)` |
| 3067,3095,3249,3296,3321,3345,3381,3404,3499,3630,3684,3742 | `ur_24_a_release_*`、`ur_29_a_lost_*`、`ur_24_a_segment_*`（各テスト名は grep のとおり） | `idle = false` | `search(k, true)` |
| 3272,3590,3712 | `ur_24_a_release_for_a_refused_transmit_segment_applies_nothing`、`ur_24_a_segment_booked_while_a_command_waits_for_a_channel_holds_it_at_its_instant`、`ur_24_a_release_after_a_transmit_channel_leaves_keeps_the_setting_until_its_instant` | `idle = false`（Tx） | `search(k, true)` |

`tests/fake.rs`・`tests/hardware.rs` は `StreamState` の印を直接読まない（`manifest.events.delivered` は別）。
`tests/fake.rs` の `ur_26_two_receive_streams_end_at_their_own_stops` はそのまま残す。

## 5. 食い違い（メモ R2 の表と HEAD のコード）

1. **TX の初期化。** メモは「`start` が clock を持てば `Active`」だが、HEAD の `Control::new` は
   `idle = false`・`active = None`（= `Pending`）にし、最初の `plan_section` が `Active` にする。
   HEAD を正とし、`starting(tx, true)` は `Pending`。観測は変わらない（`is_idle` は false、`active` は None、
   `claim_release` は拒む）が、この段階では表現も変えない。N1 の 4 が固定する。
2. **RX の初期化の 2 段。** メモは「初期化」を 1 行で `Active` と書くが、HEAD は `Control::new` が
   `Pending`（T0 あり）／`Idle`（なし）にし、その直後の `Rx::new` が `Active` にする。
   `Provider::start` は RX を 1 本ずつ作り、そのたびに spawn する（`provider/mod.rs:633-664`）ので、
   先行する owner がこの stream の `Rx::new` より前に T0 前の loss を publish できる。2 段をそのまま写し、
   その間の loss を N3 が実際に作る。HEAD の 2 段をそのまま写す（`starting` と `started`）。
3. **`delivered` の tuple。** メモの `Active { delivered: Option<i64> }` は HEAD の
   `delivered: Option<(i64, Config, i64)>` の origin・config を落とす。HEAD の tuple をそのまま残す。
   `segment: None` は T0 の segment が start の途中の loss で plan から消えたときに到達し、HEAD は floor の tuple を
   保つ（`first_undelivered` は `Some`）。End が floor を適用するかは `cuts` が `line.made(...).cut.is_none()` で
   決めるので、消えた segment では適用されない。N2・N3 が tuple を、N3 と J54/P9 が End の同値を固定する。
   メモの形に合わせて tuple を落とす案は採らない（P1b の状態を表せない）。
4. **`plan_section` で clock があり `plan.get(at)` が None。** メモは「今の segment で `Active`」だが、
   HEAD は `Active { segment: None }`。N6 の 2 がこの枝を固定する（`active()` は None、`is_idle` は false、
   `finished` は None を返す）。
5. **`search` は前の `Active` を消す。** HEAD は `idle` だけを変えて `active` を残せたが、phase は 1 値なので
   `search` は `Pending`/`Idle` に置き換える。`begin` は stream が無いときだけ走るので production では到達不能。
   N4 の 3 が固定する。

## 6. mutation 行の一覧と当て直し

`plan/maintenance/tools/mutations.json` を `old` / `new` の `\.(idle|active|delivered)\b` で grep した結果
（`Item.delivered` の literal と他 crate の `delivered` を含む）。R2 で触る行：

| 行 | 今の file / test_bin / filter | 今の old → new | 当て直し |
|---|---|---|---|
| J46 | `core.rs` / lib / `ur_29_a_loss_ends_the_stream_at_its_instant` | `item.delivered = item.delivered.or(delivered.filter(...))` → `let _ = (delivered, cuts);` | **そのまま**。`book` は `let delivered = self.first_undelivered(stream);` に変わるが、local 名 `delivered` と filter/map の行は変えない |
| J54 | `core.rs` / lib / `ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts` | `delivered.filter(\|(origin, config, _)\| cuts(*origin, *config))` → `delivered.filter(\|_\| { let _ = &cuts; true })` | **そのまま**（J46 と同じ理由） |
| P258 | `rx.rs` / lib / `ur_29_a_loss_ends_the_stream_at_its_instant` | `streams.stream_mut(...).delivered = Some((clock.origin, stream.config, k + len));` → 同じで `k` | old/new を `streams.delivered(stream_key(Dir::Rx, self.k), clock.origin, stream.config, k + len);` → `..., k);` に |
| P404 | `tx.rs` / lib / `ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call` | old は holder の early return、new は `streams.stream_mut(key).idle = false;` | old はそのまま。new を `streams.search(key, true);` に |
| P445 | `core.rs` / lib / `ur_26_an_end_takes_delivered_from_each_segment_s_own_stream` | `let delivered = self.stream(stream).delivered;` → `...booked_stream...` | old/new を `let delivered = self.first_undelivered(stream);` → `...booked_stream(dir_of(stream))...` に |
| P470 | `rx.rs` → `core.rs` / lib / `ur_26_a_stream_s_delivered_is_cleared_alone` | `streams.stream_mut(key).delivered = None;` → 全 RX stream の `delivered = None` | `Streams::finished` の本体に移す。old を `let active = std::mem::replace(phase, Phase::Pending);`、new をその後に全 RX stream の `phase = Phase::Pending` にする loop。`active.active()` は残す |
| P472 | `rx.rs` → `core.rs` / lib / `ur_26_a_finished_stream_clears_only_its_own_floor`（N7） | P470 と同じ | 別の変異にする：`let active = std::mem::replace(phase, Phase::Pending);` → 置き換えをやめ `let active = Phase::Pending;`（`phase` を消さない）。N7 の「rx1 の floor が消える」が落とす。fake のテストは残す |
| P382 | `rx.rs` / lib / `ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing` | old の comment に `the idle mark`、new は `if false` | comment を phase の言葉に書き換えるので、old を新しい comment の block に当て直す（new の `if false` はそのまま） |

**配線を殺す行（新規）**：`Rx::finish` の `if let Some(active) = streams.finished(key) { streams.complete(key, active); }` を
`if false { let _ = streams.finished(key); }` に。filter は `ur_26_a_stream_s_delivered_is_cleared_alone`。
これが無いと、`finished` 本体の変異は殺せても「`Rx::finish` が `finished` を呼ぶこと」は殺せない。

**新しい遷移の行（新規、コードと一緒に足した）**：

| id | 変異 | filter |
|---|---|---|
| P552 | `starting`：`held` のとき `Idle`、でなければ `Pending` | `ur_25_a_stream_starts_idle_or_waiting` |
| P553 | `search`：`found` のとき `Idle`、でなければ `Pending` | `ur_25_a_search_leaves_the_stream_pending_or_idle` |
| P554 | `started`：`delivered: Some((origin, config, 0))` → `delivered: None` | `ur_17_a_receive_segment_moves_from_pending_to_active_to_pending` |
| P555 | `delivered`：書く値を `k` → `k - 1` | `ur_17_a_receive_segment_moves_from_pending_to_active_to_pending` |
| P556 | `finished`：`Phase::Pending` → `Phase::Active { segment: None, delivered: None }` | `ur_17_a_receive_segment_moves_from_pending_to_active_to_pending` |
| P557 | `plan_read`：`clock` と `switch` の枝を入れ替える | `ur_25_a_transmit_read_sets_idle_pending_or_active` |
| P558 | `first_undelivered`：`self.phase(stream).delivered()` → `None` | `ur_26_a_finished_stream_clears_only_its_own_floor` |
| P559 | `Rx::finish` の配線（上記） | `ur_26_a_stream_s_delivered_is_cleared_alone` |
| P560 | `plan_read`：clock があっても `Active` の `segment` を `None` にする | `ur_25_a_transmit_read_sets_idle_pending_or_active` |

触らない行（old/new の grep に出るが、R2 の変更の外）：

- `J11`・`P357`・`P503` は `Item { ... delivered: None ... }` の literal。`book` の呼び出しは変えない。
- `P252`・`P253`・`P496` は `wants_from_none` の local `idle`。`let (old, idle, planned) = ...` の読みだけを
  `is_idle` に変え、local 名 `idle` は残す（old の行は `streams.stream(stream).idle` を含まない）。
- `P145`・`F29`・`F37` は `ezsdr-mock-radio` の `delivered`、`J284`・`J294`・`J312` は host の busy/idle、
  `P489`・`P508` は `needed_before`。いずれも R2 の印ではない。

## 7. 実行とゲート（実装段階で）

- `cargo test -p ezsdr-radio-uhd --lib`（N1–N7・N8 と既存 unit）。
- `cargo test -p ezsdr-radio-uhd --test fake`（P10・P11 と end-to-end）。
- 既存の thread テスト（P2・P3・P4・P6・P7）はそのまま。
- mutation は AGENTS.md §7 のとおり `remote_mutate.py` で、fix round は触った行だけ、gate で全リスト。
- `cargo fmt` / `rustfmt` は流さない。tree の外の写しで試す場合も同じ。
- 実装後、`grep -rnE "\.(idle|active|delivered)\b"` が test で 0（`Item.delivered`・
  `manifest.events.delivered` を除く）になることを確かめる。`phase` の直接読みが core.rs の
  transition の外に残らないことも見る。

## 8. リスクと未決

- **観測の弱いテスト。** N1–N8 は phase の読みが主で、production の device call は見ない。
  P2–P7 の既存テストが device call と cursor を見るので、遷移の配線（誰がどの関数を呼ぶか）は
  そちらが受け持つ。N シリーズだけで「配線が正しい」とは主張しない。
- **`phase` をテストに公開する。** private field のまま `phase()` を `pub(crate)` にする。
  production の読みは `is_idle`・`active`・`first_undelivered` に限る（review で grep する）。
- **`finished` の戻り値。** `Rx::finish` と `Tx::do_switch` の両方が `complete` に渡す。
  片方だけ `finished` にすると `completed` が欠け、`needed_before` が move の解放を待ち続ける（P10）。
- **P472 の当て直し。** N7 は `Streams` の unit なので、`Rx::finish` の呼び出し配線（`finished` を呼ぶこと）
  は見ない。配線は P470 の `ur_26_a_stream_s_delivered_is_cleared_alone` と P11 の fake テストが見る。
- **メモとの差 1–5。** 実装は HEAD を取り、メモの R2 の表の文言はこの段階では直さない
  （design 18 の UR-17・UR-25 の書き換えで phase の言葉にする）。

## 9. 修正（Sol の計画レビュー）

`sol-plan.log` の最後の指摘（すべて orchestrator が採用）への修正。元の計画はこの節の前に直した。

1. **P6（major）**：既存の `ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch` の最後は 3 個目の
   segment の `Active`（`Idle` ではない）。既存テストはそう直し、switch 元の `completed` も見る。
   別に N8 `ur_25_a_refused_last_transmit_change_ends_idle` を足し、後続の無い拒否で旧 segment が完了し、
   clock が無く、最後が `Idle` であることを見る。
2. **P4（major）**：拒否からの再開は command 2 の `Start` なので、再開後の `active` は `by: Some(2)` を見る。
   configure 中の loss の既存テストにも、戻った直後が `Pending`、active も floor も無いことを足した。
3. **P1b（major）**：N3 は line を最初から置かないのではなく、T0 の line を作り、T0 前の loss を実際に book して
   segment を消してから `Rx::new`・`finish` を通す。phase、完了記録なし、loss の `End` と cut が変わらないことを見る。
   §5 の「その間はどの thread も動かない」は誤り（`Provider::start` は RX を 1 本ずつ作り、そのたびに spawn する、
   `provider/mod.rs:633-664`）なので、§5 の 2 を訂正した。
4. **N7（minor）**：Orderly ではなく Abort を使い、End の instant を stream 0 の delivered（3 000）より後に置く
   （800 000 root tick = sample 4 000）。stream 0 が instant ではなく delivered で切れることを見る。
5. **N4・N5（minor）**：N4 の 3 で `Active` から `search(false)` を 1 回呼び、`Active` が消えることを見る。
   N5 の 2 で `Pending` 中に `delivered` を呼び、書かれないことを見る。どちらも主張どおりに殺せる。
6. **tuple の削減（その他）**：`segment: None` のときも HEAD の tuple を残し、End の同値を N3 と J54/P9 で固定する。
   §5 の 3 をそのように直した（元の計画は tuple を落とす案だった）。
7. **N6（その他）**：clock があり plan に segment が無い枝を N6 の 2 で見る。§5 の 4 もそのように直した。

## 10. 実装後の記録（この計画を書いた後に足した）

- コード：`Phase`・`Segment`・6 遷移・4 読みを `core.rs` に、呼び出しを `control.rs`・`rx.rs`・`tx.rs` に。
  `StreamState` の `idle`・`active`・`delivered` は消え、private な `phase` だけが残る。
- テスト：N1–N8 を足し、P1・P3・P4・P5・P6・P7・P8・P9 の既存テストに phase/completed の assert を足した。
  §4 の置き換えはすべて行った（`grep` で production の直接の読み書きは 0）。
- 結果：`cargo test -p ezsdr-radio-uhd --lib` 259 passed、`--test fake` 264 passed（2026-10-22）。
- mutation 行：§6 の当て直しと新規行を `mutations.json` に入れ、§7 の gate で回した。当て直し・新規と、`idle`/`active`/`delivered` を old/new に持つ既存行の 27 行はすべて killed（`sim02:3,sim03:3`、`REMOTE_CPUS=8`、2026-10-22）。
- セルフレビュー 1 回目（同じモデル、P1 なし）の P2/P3 を直した：UR-25 の phase 定義に `segment: None` と RX だけの `delivered`、`finish` 後の `Pending` を足した。N8 の cut を `c0.at_or_after(e)` に、N3 の clock の node を `core.stream_node(Dir::Rx, 0)` に直した。Changes の record を memo 35 への link にした。
