# R1 テスト計画：UHD Module の stream ごとの holder と読み

2026-10-22、実装段階 1b-6a（`plan/phase8/35-uhd-stream-state.md` §2 R1）の risky step 2。
対象は `crates/ezsdr-radio-uhd/src/provider/` の `StreamState` の `busy`・`busy_seq`・`reading` を、
private な `Claims { holder: Holder, reading: bool }` と遷移の関数に閉じ込めることだけ。
`idle`・`active`・`delivered`（R2）、device の記録、loss、#67 は変えない。
この文書は計画であり、コード・テスト・mutation 行はまだ書いていない。実行もしていない。

## 1. 対象の関数と観測

実装で置く型と関数（名前と signature の案。`Claim` は `#[must_use]`）。

```rust
pub(crate) struct Claims { holder: Holder, reading: bool }      // field は private
pub(crate) enum Holder { Free, Owner { seq: Option<u64> }, Release, FromNone }
pub(crate) struct Claim { stream: StreamKey, holder: Holder }  // stream と取った役割

impl Streams {
    pub fn claim_owner(&mut self, stream: StreamKey, seq: Option<u64>) -> Option<Claim>;
    pub fn claim_release(&mut self, stream: StreamKey) -> Option<Claim>;
    pub fn claim_from_none(&mut self, stream: StreamKey) -> Option<Claim>;
    pub fn may_call_from_none(&self, stream: StreamKey) -> bool;
    pub fn begin_read(&mut self, stream: StreamKey) -> bool;
    pub fn begin_shutdown_read(&mut self, stream: StreamKey);
    pub fn end_read(&mut self, stream: StreamKey);
    pub fn give_back(&mut self, claim: Claim);
    pub fn holder(&self, stream: StreamKey) -> Holder;   // 読むだけ
    pub fn reading(&self, stream: StreamKey) -> bool;    // 読むだけ
}
```

`move_source_busy(stream, seq)` は signature を変えず、`Owner { seq }` を読む。

`Claim` の Drop は **test build でだけ** panic する（返されずに落ちたことを知らせる）。
production では何もしない。`give_back` は claim を消費するので、`give_back` の中で
`std::mem::forget` するか、`returned` の印を立ててから Drop させる。ここを忘れると
`give_back` の正常終了時に Drop が panic する（test build）。mismatch の panic で二重 panic に
しないため、mismatch を判定したら claim を `mem::forget` してから panic する。

`end_read` と `give_back` の「前が違うとき」は、設計メモは test build の panic だけを決めている。
production の扱いは §7 の未決 A に置く（推奨：状態を変えない）。

### 1.1 遷移の表（テストが固定する内容）

| 関数 | 前 | 後 | 前が違うとき | 主なテスト |
|---|---|---|---|---|
| `claim_owner` | `Free` | `Owner { seq }` | `None`、holder は不変 | N1 |
| `claim_release` | `Free`、`idle`、`active` なし | `Release` | `None` | N2 |
| `claim_from_none` | `Free`（`reading` はどちらでもよい） | `FromNone` | `None` | N3 |
| `may_call_from_none` | `FromNone`、`reading = false` | `true` | `false`（読むのは待つ） | N3 |
| `begin_read` | `Free`、`reading = false` | `reading = true`、`true` | `false`、状態不変 | N4 |
| `begin_shutdown_read` | どれでも | `reading = true` | —（holder を見ない） | N5 |
| `end_read` | `reading = true` | `reading = false` | test build で panic | N6 |
| `give_back` | claim が取った holder | `Free`（`reading` はそのまま） | test build で panic | N7、N8 |
| `move_source_busy` | `Owner { seq }` が同じ move の source | `true` | `false` | N9 |
| `holder` / `reading` | — | 現在の値 | — | すべての観測 |

## 2. 系列 S1–S8 と既存テスト

「今あるテスト」は HEAD `8cd89e4` の名前。行番号は動くので目安。

| 系列 | 今あるテスト（何を assert しているか） | 新しいテスト／変更 |
|---|---|---|
| S1 owner が Free を取り、configure 成功で返す | `ur_25_an_owner_begins_a_from_none_enable_without_a_second_open`（control.rs:1869）: enable の後、owner は再 open も再 configure もせず `rx_start` だけ。`ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer`（tests/fake.rs:4906）: arm の streamer を最初の segment がそのまま使う。`ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing`（tx.rs:2180）の booking 後: `tx.read`/`tx.step` が通り、`tx_open` は 1 回 | N1（claim の往復）。成功後に holder が `Free` であることは、更新する `rx_record`/`tx_record` と各テストの最終 assert が見る |
| S2 owner の configure 拒否。拒否を book してから Free | `ur_29_a_lost_receive_owner_configure_releases_stream_lock_before_reporting`（rx.rs:736）: lost configure の後 `!busy`、`rx_start` なし、DEVICE_LOST。`ur_29_a_lost_transmit_owner_configure_releases_stream_lock_before_reporting`（tx.rs:2498）: lost configure の後 `!busy`、clock なし、`tx_send` なし。`ur_25_a_refused_segment_is_booked_by_uhd_rx`（rx.rs:1434）: 拒否が line に載り、次の begin が続く。`ur_13_a_failed_configure_by_the_owner_is_configured_again_without_reopen`（tx.rs:1272）: configure 失敗後、次の switch が再 open せず configure する（claim が返らないと次が busy で止まる） | N10（receive の拒否が refuse と give_back で 1 回の line lock）。Tx 側は同型なので N10 の対称として review し、必要なら N10 に Tx case を足す |
| S2m move の source の configure/reopen 拒否。両 line に拒否が載るまで `Owner { seq }`、destination は開かない | `ur_34_a_waited_open_waits_for_same_move_source_configuration`（core.rs:1264）: source が `busy_seq = seq` の間 destination は開かず、refusal 後は中断。`ur_34_a_move_destination_waits_for_source_busy_after_channel_release`（core.rs:1301）: channel を離した後も source の move configure 中は destination が開かない。`ur_34_a_move_{destination_open,source_reopen,destination_configure,source_configure}_refusal_halts_both_lines_and_recovers`（tests/fake.rs:4880–4903）: 両 line が halt し、後の cold で回復 | N9（`move_source_busy` が `Owner { seq }` だけを見る）。既存の 2 テストは claim を保持する形に直す（§4） |
| S3 owner が `FromNone` を見て何もしない | `ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing`（control.rs:2702）: `rx.begin` が false、cursor と idle と device call が不変。`ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing`（tx.rs:2180）: `tx.read` が false、`at`・switch・idle・device が不変。`ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call`（tx.rs:2220）: `tx.pass` が `tx_async` も device call もしない。`ur_25_an_enable_behind_an_owner_s_open_is_a_cold_change`（control.rs:2820）: 逆方向（owner が持ち、enable が cold として book） | 変更なし（観測を `holder` に） |
| S4a 読みの最中に from-none が `FromNone` を取る | `ur_25_a_from_none_enable_waits_for_a_transmit_report_read`（tx.rs:2254）: `tx_async` を gate したまま enable が `tx_open` を呼ばず、read が終わってから book する。`ur_25_a_transmit_report_read_of_a_later_stream_does_not_hold_the_earlier_owner`（tx.rs:2425）: stream 1 の read だけを待ち、stream 0 は動く | 既存の前者を拡張：待っている間 `holder == FromNone`、`reading == true`、`tx_open == 0`。後者は stream 1 の `begin_read`/`end_read` に置換（§4） |
| S4b from-none が先に `FromNone` を取り、後から uhd-tx が読む | `ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call`（tx.rs:2220）: `tx.read` が false、`idle` 不変、`tx_async` 0、`reading` false | 既存を拡張：`holder == FromNone` を assert。N4 が `begin_read(FromNone) == false` を固定 |
| S5 from-none の読み待ちに stop か loss | `ur_25_a_from_none_enable_ends_its_wait_at_the_provider_stop`（tx.rs:2301）: stop が待ちを終わらせ、device call も book もしない。`ur_29_a_lost_device_ends_a_from_none_enable_s_wait_for_a_report_read`（tx.rs:2346）: loss が同じ。`ur_26_an_enable_from_no_stream_a_stop_recorded_after_its_open_books_nothing`（control.rs:2954）: open 後の stop でも book しない | 既存の 2 つを拡張：control が holder を返した後も `reading == true` で、tx の `end_read` で false。`holder == Free` も assert |
| S6 `FromNone` のまま shutdown の読み | `ur_25_a_detached_control_does_not_hold_the_shutdown_s_read`（tx.rs:2566）: `busy` を立てたまま shutdown が `tx_async` を読む | 既存を `claim_from_none` + `mem::forget`（detach の模型）に変更。T1/T2 を追加（両順序） |
| S7 `release_unneeded` が Free・idle だけを `Release` で取る | `ur_34_a_release_skips_a_stream_held_busy_by_from_none`（core.rs:1351）: `FromNone` の stream は閉じない。`ur_34_a_release_closes_an_idle_streamer_needed_by_a_move`（core.rs:1367）: idle で必要な streamer を閉じる。`ur_34_a_release_keeps_an_idle_streamer_with_no_peer_need`（core.rs:1376）: 不要なら閉じない。`ur_34_a_waited_open_waits_for_a_finite_unprocessed_move_segment`（core.rs:1214）: finite segment の完了まで待つ | N2（`claim_release` の前提と他 holder）。既存は観測だけ置換 |
| S8 返し忘れ | 各道の後で `!busy` を見ているテスト（S2/S5/S6/S7 のもの）。返し忘れが残ると holder が `Free` にならず落ちる | N7（Drop の panic）、N6（`end_read`）、N8（mismatch）。`#[must_use]` は無視を警告にする |

## 3. 新しいテスト

### 3.1 thread を使わない `Streams` の unit test（`core.rs` の `mod tests`）

#### N1 `ur_25_a_claim_owner_takes_only_a_free_stream`

- 目的：S1 と、`claim_owner` の「前が違うとき」。
- 準備：`Streams::with_streams(1, 1)`、`k = stream_key(Dir::Rx, 0)`。
- 手順と assert：
  1. `holder(k) == Free`。`claim_owner(k, None)` が `Some`。`holder(k) == Owner { seq: None }`。
  2. `claim_owner(k, Some(7))` は `None`、`claim_from_none(k)` も `claim_release(k)` も `None`、holder は `Owner { seq: None }` のまま。
  3. `give_back(claim)`。`holder(k) == Free`。
  4. `claim_owner(k, Some(9))` が `Some`、`holder(k) == Owner { seq: Some(9) }`。`move_source_busy` は move が無いので false（N9 で move を用意）。
  5. `give_back`。`Free`。
- 観測：戻り値と `holder` accessor。cursor・idle・device は `Rx::begin`/`Tx` の既存 S3 テストが見る。
- 落とす誤実装：`Free` 以外でも claim を返す（2 が `Some`）、holder を立てない（1 が `Free`）、`give_back` が `Free` にしない（3）、`Owner { seq }` の seq を取り違える。

#### N2 `ur_25_a_claim_release_needs_a_free_idle_stream_with_no_active_segment`

- 目的：S7 の前提（`claim_release` が `Free`・`idle`・`active` なしだけを取る）。
- 準備：`Streams::with_streams(1,1)`、`k`。`idle = true`、`active = None`。
- 手順と assert：
  1. `claim_release(k)` が `Some`、holder `Release`。他の `claim_owner`/`claim_from_none`/`claim_release` は `None`。
  2. `give_back` して `Free`。
  3. `idle = false` にすると `claim_release(k)` は `None`、holder は `Free` のまま。
  4. `idle = true`、`active = Some((0, Config::default(), None))` にすると `None`。
  5. `active = None`、`idle = true` に戻すと `Some`、`give_back`。
- 観測：戻り値と `holder`。`set` が `Open` でない、`needed_before` が false の場合は `release_unneeded` 側の既存テスト（S7）が見る。
- 落とす誤実装：`idle` を見ない（3 が `Some`）、`active` を見ない（4 が `Some`）、`Free` 以外でも取る（1 の他 claim）。

#### N3 `ur_25_a_claim_from_none_admits_a_read_in_flight`

- 目的：S4a の状態（`FromNone` は読みの最中でも取れるが、device を呼ぶのは読みの後）と `may_call_from_none`。
- 準備：`Streams::with_streams(1,1)`、`k`。
- 手順と assert：
  1. `begin_read(k)` が `true`（`Free`、`reading` false）。
  2. `claim_from_none(k)` が `Some`（`reading == true` でも取れる）。holder `FromNone`。
  3. `may_call_from_none(k) == false`（読みが進行中）。
  4. `end_read(k)`。`may_call_from_none(k) == true`。
  5. `give_back`。holder `Free`、`reading == false`。
  6. 別の `Streams` で `claim_from_none` を取り、`begin_read(k)` が false（holder が `FromNone`）、`reading` は false のまま。`give_back`。
- 観測：戻り値と `holder`/`reading`。
- 落とす誤実装：`claim_from_none` が `reading` を理由に `None`（2 が失敗 → S4a の待ちが成立しない）、`may_call_from_none` が `reading` を見ない（3 が true → read 中の device call）、`begin_read` が holder を見ない（6 が true）。

#### N4 `ur_25_a_begin_read_needs_a_free_stream_and_no_read`

- 目的：S4b と `begin_read` の全前提。
- 準備：`Streams::with_streams(1,1)`、`k`。
- 手順と assert：
  1. `begin_read(k) == true`、`reading(k) == true`、holder `Free`。
  2. 二度目の `begin_read(k) == false`、`reading` は true のまま。
  3. `end_read(k)`、`reading == false`。
  4. `claim_owner(k, None)` を取り、`begin_read(k) == false`。`give_back`。
  5. `claim_from_none(k)` を取り、`begin_read(k) == false`（S4b）。`give_back`。
  6. `claim_release(k)`（idle を true に）を取り、`begin_read(k) == false`。`give_back`。
- 観測：戻り値と `reading`/`holder`。
- 落とす誤実装：holder を見ない（4/5/6 が true）、`reading` を見ない（2 が true → 二重 read）、`begin_read` が失敗時に `reading` を立てる。

#### N5 `ur_25_a_begin_shutdown_read_ignores_the_holder`

- 目的：S6 の例外（`shutdown` の読みは holder を見ない）。
- 準備：`Streams::with_streams(1,1)`、`k`。
- 手順と assert（各 holder を順に）：
  1. `Free`、`reading = false`：`begin_shutdown_read(k)`、`reading == true`、holder `Free`。`end_read`。
  2. `Owner { seq: Some(3) }` を claim して：`begin_shutdown_read`、`reading == true`、holder `Owner` のまま。`end_read`、`give_back`。
  3. `FromNone` を claim して：`begin_shutdown_read`、`reading == true`、holder `FromNone` のまま。`end_read`、`give_back`。
  4. `Release` を claim して：同じ。
  5. `FromNone` で既に `reading = true` のとき `begin_shutdown_read` を呼んでも `reading == true` のまま（panic しない）。`end_read`、`give_back`。
- 観測：`reading`/`holder`。
- 落とす誤実装：`begin_shutdown_read` が `Free` を要求する（2–4 が false/panic）、holder を変える、`reading` を立てない（1）。

#### N6 `ur_25_an_end_read_without_a_read_panics_in_the_test_build`

- 目的：`end_read` の「前が違うとき」。
- 手順：`Streams::with_streams(1,1)` で `end_read(k)` を呼ぶ。
- 観測：`#[should_panic(expected = "...")]`。panic の文言は実装で決め、このテストと `Claim` の文言を一致させる。
- 落とす誤実装：`end_read` が false でも黙って通る（テストが panic しない）。production は panic しない（§7 未決 A）。

#### N7 `ur_25_a_claim_dropped_without_give_back_panics_in_the_test_build`

- 目的：S8 の見張り。返し忘れの道をテストが見つける。
- 手順：`Streams::with_streams(1,1)` で `claim_owner(k, None)` を取り、`give_back` せずに drop する。
- 観測：`#[should_panic(expected = "...")]`。Drop は lock を取らない。
- 落とす誤実装：Drop が何もしない（テストが通る）、Drop が lock を取り直す（deadlock/別の失敗）。

#### N8 `ur_25_a_give_back_of_another_holder_panics_in_the_test_build`

- 目的：`give_back` の「前が違うとき」。public な遷移だけでは mismatch を作れないので、`core.rs` の test module が `Claim { stream: k, holder: Holder::Free }` を直接作る（field は同じ module から見える）。
- 手順：`Streams` で `claim_owner(k, None)` を取って holder を `Owner` にし、`Claim { stream: k, holder: Holder::Free }` を `give_back` に渡す。
- 観測：`#[should_panic]`。`give_back` は mismatch を判定したら claim を `mem::forget` してから panic し、二重 panic にしない。
- 落とす誤実装：mismatch でも holder を `Free` にする（panic しない）、二重 panic で abort する（テストが失敗ではなく abort）。

#### N9 `ur_25_move_source_busy_reads_the_owner_of_the_move`

- 目的：`move_source_busy` が `Owner { seq }` だけを読むこと（P505/P512 の土台）。
- 準備：`Streams::with_streams(1, 2)`、`from = stream_key(Dir::Rx,0)`、`to = stream_key(Dir::Rx,1)`、`lines.book_move(from, to, 0, Some(10_000), 0, 7, 0)`。
- 手順と assert：
  1. `claim_owner(from, Some(7))`。`move_source_busy(to, Some(7)) == true`。
  2. `claim_owner` の seq が違う（`Some(8)` を別の stream で、または holder を `Owner { Some(8) }` にする）と false。
  3. `claim_from_none(from)` / `claim_release(from)` では false。
  4. `give_back` 後は false。
- 観測：戻り値。move の line は `lines.book_move` が用意する。
- 落とす誤実装：`holder != Free` を busy とみなす（3 が true）、seq を見ない（2 が true）、`busy_seq` の代わりに別の値を見る。

### 3.2 line lock の節（S2）

#### N10 `ur_25_a_refused_receive_owner_books_and_gives_back_in_one_line_section`

- 目的：S2 の「拒否を book してから Free。book の前に他の thread が Free を見ない」を、1 つの line-lock 節として固定する。
- 準備：`rig_with`（`FakeFault::RxApplyFails` など、`Owner::Configure` になる plan。`release_first` が false になるよう `set` は同じで `configured` だけ違う）と `Rx`。
- 手順：
  1. `record_locks()` を呼び、`rx.begin()` を 1 回実行する（拒否される segment）。
  2. `taken_locks()` を `core.streams` のアドレスで filter し、**1 節**（この経路では 4 回：begin の plan 読み、owner_configure の初回、configure 失敗の記録、呼び出し側の refuse+give_back）であることを assert する。実装時に数え直し、値と理由をコードのコメントに残す。
  3. line に `refused` が載り、`holder(k) == Free`、`reading == false`。
- 観測：`note_lock`/`taken_locks`（test build の既存機構）。1 節なら他の thread は間に入れない。現在の 5 回（refuse と clear が別）は落ちる。
- 落とす誤実装：refuse の後に guard を落として give_back を別の節にする（5 回）、give_back を忘れる（holder が `Owner` のまま、Drop panic）。
- Tx の `do_switch` は同じ形だが、経路の lock 数がテストごとに変わる。まず N10 を置き、Tx 側は review で確かめる。必要なら `ur_25_a_refused_transmit_switch...` に同じ数え方の case を足す（実装時に判断）。

### 3.3 thread と probe（S6）

T1/T2 は `spy_rig()`（`LockSpy`、manual clock）を使う。uhd-control は **raw `std::thread::spawn`**（participant にしない。manual clock を止めないため）、shutdown も raw thread。device call は `spy.stop_before("apply")` で control を止め、`spy.stop_before("tx_async")` で shutdown の読みを止める。`apply` は `family` が `None` なので `tx_async` と衝突しない。

#### T1 `ur_25_a_shutdown_read_and_a_returning_from_none_resolve_with_the_control_first`

- 目的：S6 の順序 (a)。`FromNone` のまま control が device call で止まっている間に shutdown の読みが始まり、control が先に `give_back` する。`reading` は残り、最後に両方解ける。
- 手順：
  1. `control_of(&core)` を取り、raw thread で `tx_channels_from_none(&mut control, &core, 2, 100, &[0])` を実行。`spy.stop_before("apply")` で止める。
  2. `gate.wait()`。`holder == FromNone`。
  3. `tx = Tx::new(...)` を raw thread で `tx.shutdown()`。`spy.stop_before("tx_async")` で読みを止める。
  4. 読みの gate を待つ。`reading == true`、holder `FromNone`。
  5. control の gate を release し、control thread を join。`holder == Free`、`reading == true`（shutdown の読みはまだ飛んでいる）。`spy.seen("tx_open") == 1`、`apply >= 1`。
  6. shutdown の gate を release し、shutdown thread を join。`reading == false`、holder `Free`。
- 観測：`holder`/`reading` accessor、`spy.seen`、join。manual clock は raw thread が participant でないので進む（M6）。
- 落とす誤実装：`give_back` が `reading` も消す（5 で `reading == false`）、`end_read` が holder を返す（5 で `Free` でなくなる）、`begin_shutdown_read` が holder を見て待つ（4 に到達しない）、control が読みの前に device を呼ぶ（`tx_open`/`apply` の順序）。

#### T2 `ur_25_a_shutdown_read_and_a_returning_from_none_resolve_with_the_read_first`

- 目的：S6 の順序 (b)。shutdown の読みが先に終わり、`holder` は `FromNone` のまま残る。その後 control が `give_back` して両方解ける。
- 手順：
  1. T1 の 1–4 と同じ（control は `apply` で停止、shutdown は `tx_async` で停止、`holder == FromNone`、`reading == true`）。
  2. shutdown の gate を release し、shutdown thread を join。`reading == false`、holder `FromNone` のまま。
  3. control の gate を release し、control thread を join。`holder == Free`、`reading == false`。
- 観測：同じ。
- 落とす誤実装：`end_read` が holder を `Free` にする（2 で `Free`）、`begin_shutdown_read` が読みの間に holder を返す、control の `give_back` が消える（3 で `FromNone` のまま、Drop panic も見張る）。

## 4. 印を直接読み書きしている既存テストと置き換え

`grep -rnE "\.(busy|busy_seq|reading)\b" crates/ezsdr-radio-uhd` の test build の該当箇所。
`tests/fake.rs:3677` の `plan.busy()` は別の型（watchdog の plan）なので対象外。

| 場所 | テスト／helper | 今 | 置き換え |
|---|---|---|---|
| control.rs:2698 | `rx_record` | `(state.busy, state.reading, ...)` | `(streams.holder(k) != Holder::Free, streams.reading(k), ...)`。返す tuple の形は変えない |
| control.rs:2798 | `ur_25_a_from_none_holds_the_streamer...` の probe | `...stream(...).busy` | `...holder(...) == Holder::FromNone` |
| control.rs:2805 | 同上 `cold_booking` probe | `streams.expect(...).stream(...).busy` | `streams.expect(...).holder(...) == Holder::FromNone` |
| tx.rs:2176 | `tx_record` | `(state.busy, state.reading, ...)` | `(streams.holder(k) != Holder::Free, streams.reading(k), ...)` |
| tx.rs:2437 | `ur_25_a_transmit_report_read_of_a_later_stream...` | `stream_mut(k1).reading = true` | `let read = lock(...); assert!(read.begin_read(k1)); drop(read);`（Drop guard で `end_read(k1)`） |
| tx.rs:2442 | 同 Drop guard | `stream_mut(k1).reading = false` | `lock(...).end_read(k1)` |
| tx.rs:2470 | 同 assert | `stream(k1).reading` | `streams.reading(k1)` |
| tx.rs:2509 | `ur_29_a_lost_transmit_owner_configure...` | `!state.busy` | `streams.holder(k) == Holder::Free` |
| tx.rs:2572 | `ur_25_a_detached_control_does_not_hold_the_shutdown_s_read` | `state.busy = true` | `let claim = lock(...).claim_from_none(k).expect(...); std::mem::forget(claim);`（detach の模型。Drop させない） |
| core.rs:1272–1273 | `ur_34_a_waited_open_waits_for_same_move_source_configuration` | `source.busy = true; source.busy_seq = Some(seq)` | `let claim = streams.claim_owner(from, Some(seq)).expect(...);` を保持 |
| core.rs:1288–1289 | 同 | `busy = false; busy_seq = None` | `streams.give_back(claim)` |
| core.rs:1325–1326 | `ur_34_a_move_destination_waits_for_source_busy_after_channel_release` | `source.busy = true; busy_seq = Some(1)` | `let claim = streams.claim_owner(from, Some(1)).expect(...)` |
| core.rs:1342–1343 | 同 | clear | `streams.give_back(claim)` |
| core.rs:1358 | `ur_34_a_release_skips_a_stream_held_busy_by_from_none` | `state.busy = true` | `let claim = streams.claim_from_none(stream).expect(...); std::mem::forget(claim);` |
| core.rs:1363 | 同 | `stream(stream).busy` | `holder(stream) == Holder::FromNone` |
| core.rs:1410 | `ur_34_a_waited_open_reloads_a_move_refused_while_it_waits` | `!stream(to).busy` | `holder(to) == Holder::Free` |
| core.rs:1508 | `ur_34_a_waited_open_aborts_when_the_planned_segment_changes` | `!stream(to).busy` | 同上 |
| core.rs:1541 | `ur_34_a_waited_open_ends_at_device_loss_without_closing_after_loss` | `!stream(...).busy` | 同上 |
| core.rs:1562 | `ur_34_a_waited_open_ends_at_provider_stop_without_opening` | `!stream(...).busy` | 同上 |
| core.rs:1623 | `ur_34_a_waited_transmit_open_observes_provider_stop...` | `!stream(to).busy` | 同上 |
| rx.rs:745 | `ur_29_a_lost_receive_owner_configure...` | `!state.busy` | `holder(k) == Holder::Free` |
| rx.rs:801 | `ur_34_a_waited_receive_open_exits_when_stop_removes_its_segment` | `!stream(to).busy` | 同上 |
| tx.rs:1024 | `ur_34_a_transmit_stop_during_release_wait_cancels_later_bursts` | `!stream(to).busy` | 同上 |

`ur_25_a_from_none_leaves_the_stream_unbusy_on_every_path`、`ur_25_an_enable_from_no_stream_after_the_provider_s_stop_calls_no_device`、
`ur_26_an_enable_from_no_stream_a_stop_recorded_after_its_open_books_nothing`、`ur_29_a_lost_device_stops_a_from_none_enable_between_channels` は
`rx_record` 経由なので、helper の置き換えだけで `busy`/`reading` の意味を保つ。`ur_25_a_from_none_holds...` は probe も直す。

### 4.1 既存テストの追加 assert（S4a/S5）

- `ur_25_a_from_none_enable_waits_for_a_transmit_report_read`：待ちの間 `holder == FromNone`、`reading == true`、`tx_open == 0`。book 後 `holder == Free`、`reading == false`。
- `ur_25_a_from_none_enable_ends_its_wait_at_the_provider_stop`：control 終了後 `holder == Free`、`reading == true`。tx を止めた後 `reading == false`。
- `ur_29_a_lost_device_ends_a_from_none_enable_s_wait_for_a_report_read`：同上（loss）。
- `ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call`：`holder == FromNone`、`reading == false`。
- `ur_25_a_transmit_report_read_of_a_later_stream_does_not_hold_the_earlier_owner`：`begin_read`/`end_read` に置換。`reading` の assert を accessor に。

## 5. mutation 行の当て直し

新しい行はこの段階では書かない（コードと一緒に足す）。今の行のうち `old` か `new` に
`.busy`・`.busy_seq`・`.reading` を持つもの（`grep` の結果）。ID と filter は `plan/maintenance/tools/mutations.json` のまま。

| ID | filter（テスト） | 今の狙い | 当て直し |
|---|---|---|---|
| P252 | `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration` | enable from no stream を決して uhd-control に configure させない | 新しい `from_none` の決定（`claim_from_none` の結果）を `false` に固定し、取った claim は同じ節で `give_back` する。`let from_none = ...` の行に当てる |
| P253 | `ur_25_the_switch_applies_the_configuration_in_effect_at_e2` | すべての cold change を uhd-control が booking で configure する | `from_none` を `!channels.is_empty()` に固定する。claim が `None` のときは device call だけが先に走り、テストが落ちる形にする |
| P382 | `ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing` | rx owner が from-none の後ろで segment を始める | `rx.begin` の holder guard（`if streams.holder(key) != Holder::Free`）を `if false` にする |
| P383 | `ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call` | tx owner が from-none の後ろで plan を読む | `Tx::read` の holder guard を `if false` にする |
| P384 | `ur_25_a_from_none_holds_the_streamer_from_its_decision_to_its_booking` | from-none が streamer の所有を持たない | `let claim = streams.claim_from_none(stream);` を `None` に固定し、`from_none = false` にする（probe が `Free` を見る） |
| P385 | 同上 | booking の前に streamer を返す | `cold_booking` probe の前に `if let Some(claim) = from_none_claim.take() { streams.give_back(claim); }` を入れる。`Option<Claim>` なので表現できる。採用する（§7 未決 C） |
| P386 | 同上 | 成功の後に streamer を返さない | booking 後の `if let Some(claim) = from_none_claim { streams.give_back(claim); }` を削る |
| P389 | `ur_25_a_lost_stream_starts_no_rate_check_apply_or_booking_after_the_call` | 早期 return で所有を返さない | `if !may_call || lost || stop` の節の `give_back` を削る |
| P392 | `ur_25_a_detached_control_does_not_hold_the_shutdown_s_read` | shutdown の読みが detach された enable の所有を待つ | `if holder == Free { self.reports(...) }` に変える（`FromNone` のとき読まない） |
| P398 | `ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call` | enable の open 中も tx turn が report を読む | `Tx::pass` の holder guard を `if false` にする |
| P399 | `ur_25_a_lost_stream_starts_no_rate_check_apply_or_booking_after_the_call` | loss/stop の後に本を book する | 早期 return の条件を `if false` にし、`give_back` を残す（または book 側へ動かす） |
| P404 | `ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call` | from-none の後ろの tx owner が idle を消す | `Tx::read` の holder guard の中で `idle = false` を足す |
| P455 | `ur_25_a_transmit_report_read_of_a_later_stream_does_not_hold_the_earlier_owner` | どの stream の enable も stream 0 の読みを待つ | `(streams.reading(stream), ...)` を `(streams.reading(booked_stream(Dir::Tx)), ...)` にする |
| P496 | `ur_34_an_idle_destination_leaves_its_join_to_the_owner_when_another_streamer_holds_the_channel` | 他の streamer が channel を持つ from-none open | 新しい `from_none` の決定から `held_elsewhere` を落とす |
| P505 | `ur_34_a_move_destination_waits_for_source_busy_after_channel_release` | destination が source の move configure を待たない | 文字列は変わらない（`move_source_busy` の呼び出しのまま）。`move_source_busy` の中身だけが `Owner { seq }` を読むようになる。**当て直し不要** |
| P512 | 同上 | 同上（`release_first` の move source 待ち） | 同上。**当て直し不要** |

`J279`・`J295`・`J312` は `host.rs` の manual clock の `busy`（別の型）なので触らない。

### 5.1 一緒に確認する行（必須の grep には出ないが、テキストが変わる）

`P387`・`P388`・`P396`・`P402` は `reading` の変数・`end_reading` を参照する。
実装で `let may_call = if reading ...` の `reading` を accessor の bool にしても文言が同じならそのままでよいが、
`end_reading()` を `end_read(stream)` に変える P402 は必ず当て直す。
`P384`・`P386`・`P389`・`P399` は `busy` の代入を `give_back` に置き換えるので、同じ `old` が二度現れないよう
関数名・前後の行を含めて一意にする（`mutations.json` は exact once を要求する）。

## 6. `design/` で偽になる文（R1）

- `design/18-uhd-radio.md:265–266`（UR-25 の **Ownership** 段落）：`busy` と `reading` の名前を、
  holder（`Free`・`Owner { seq }`・`Release`・`FromNone`）と読みの進行の言葉に書き直す。
  規則（1 人の所有者、from-none の取得と返却の節、owner が device を呼ばないこと、cursor と idle を動かさないこと、
  shutdown の読みの例外、uhd-tx の report read と待ち、loss/stop の終わり方）は変えない。DA-1 の形式。
- `:269` の最後の文「`busy` marks a uhd-rx/uhd-tx owner while it changes a streamer's set, or uhd-control while it owns an enable from no stream」：
  「holder は uhd-rx/uhd-tx が set を変える間 `Owner { seq }`、uhd-control が from-none を持つ間 `FromNone`」に。
- `:271`「A source owner keeps its stream's `busy` mark until it has recorded a configuration refusal on both lines ...
  until the source owner releases the mark」：`Owner { seq }` の claim を両 line の拒否まで保持し、`give_back` で返す、に。
- `:279` の `*Test:*` の UR-25 の所有の列に、新しいテスト名（N1–N10、T1、T2 のうち UR-25 を固めるもの）を足す。
- Changes 表に 2026-10-22 の R1 の行を足す（design note 35 §2 R1 を参照。version は上げない）。
  `:645` の 2026-10-19 の行は履歴なので書き換えない。

## 7. 未決（orchestrator / Sol の判断）

- **A. `end_read`/`give_back` の mismatch の production の扱い**：設計メモは test build の panic だけを決める。
  推奨：`end_read` は `reading` が false なら何もしない、`give_back` は holder が claim と違えば状態を変えない
  （他の holder を壊さない）。test build だけ panic。N6/N8 が panic を固定する。
- **B. accessor を 2 つ足すこと**：`holder` は production の guard（rx/tx/release）に必要。`reading` は
  `await_reading` を `may_call_from_none` にするなら test 専用にできる。field は private のまま。
  最小の追加としてこの 2 つを推奨。`#[cfg(test)]` にするかは実装で決める（`holder` は production なので不可）。
- **C. P385 を残すか**：`Option<Claim>` なので「booking の前に返す」は表現できる。既存の probe テストが殺すので、
  残して当て直すことを推奨（前段の「consumes で表現できない」は `Option::take` を考慮していなかった）。
- **D. `release_unneeded` の claim 後の loss 分岐**：claim を取った後、close の前に loss になった道は、
  既存の probe では決定的に踏めない。claim を返すコードは必要だが、テストは review-only か、
  `release_claimed` のような probe を足すかの判断が要る（今回は足さない）。
- **E. S2 の注文の観測**：N10 は line lock の取得回数で「1 節」を固定する。文の順序（give_back が refuse の前か後か）は
  同じ節の中では観測できないが、同じ節なら他の thread は入れないので規則は満たす。Sol が別の観測を求めるなら、
  `Streams::refuse` に `refused` probe を 1 つ足す案がある（今回は足さない）。
- **F. `read_reports_before_close`**：`begin_read` が false のときは読みを skip する（holder が `FromNone` のとき
  device を呼ばない）。`pass` の `plan_section` の false 分岐は現状到達しないが、残すなら `end_read` を忘れない。
- **G. `tx.pass` の `begin_read` 失敗**：holder が `Free` で `reading` が false の確認と同じ節なので、失敗は
  二重 read のときだけ。失敗したら device を呼ばず `false` を返す。

## 8. 実装後の確認順（この段階では実行しない）

1. `cargo test -p ezsdr-radio-uhd --lib provider::core::tests::ur_25_`（N1–N9）
2. `cargo test -p ezsdr-radio-uhd --lib provider::control::tests::ur_25_`、`provider::tx::tests::ur_25_`、
   `provider::rx::tests::ur_`
3. `cargo test -p ezsdr-radio-uhd`（`tests/fake.rs` の move 拒否を含む）
4. `python3 plan/maintenance/tools/rules.py --base 8cd89e4`（design の edit 後）
5. mutation は実装の gate で `remote_mutate.py` のみ。この計画では流さない。

## 9. この段階でやらないこと

- コード、テスト、mutation 行を書かない。`cargo fmt`/`rustfmt`、`mutate.py`、commit/push/`git add`/`git stash` をしない。
- R2（`idle`・`active`・`delivered`）と loss、#67 を変えない。
- 指示書と設計メモに無い production の道を足さない（`Claim` の Drop の panic は例外）。

## 修正（Sol の計画レビュー）

2026-10-22、`sol-plan.log` の最後の指摘（orchestrator がすべて採用）に対する修正。この節は実装の後に書いた（「直してからコードを書く」という順序を守れなかった）。コードとテストは下の内容に従っている。

1. **S4a（major）**：`await_reading` のループの中の判定（`may_call_from_none` が false）の直後に
   `reading_wait` probe を足した。既存の `ur_25_a_from_none_enable_waits_for_a_transmit_report_read` は
   `wait_reading` ではなくこの probe で待ち、解放の前に `holder == FromNone`、`reading == true`、
   `tx_open == 0` を確かめる。probe は毎回呼ばれるので、テストの closure は `Option<Receiver>` を
   take する一度きりの gate にした。
2. **S2（major）**：`Rx::begin` と `Tx::do_switch` の拒否の節に `refused_booking` probe を足し、
   `Some(&streams)`（line lock を持つ）で呼ぶ。N10（RX は `rx.rs`、TX は `tx.rs`）は probe から
   `holder` を受け取り、`Owner { seq: Some(1) }` のままであることと、その時点で
   `core.streams.try_lock()` が失敗すること（同じ節であること）を確かめてから再開する。
   move の source の「両方の line に拒否が載るまで `Owner { seq }` を保持」は、`refuse` が両 line を
   同じ節で記録し、`give_back` がその同じ節の後にあることで満たす（N10 と同じ道）。
3. **release の後の loss（major）**：`release_unneeded` の claim の後、loss の検査の前に
   `release_claimed` probe を足した。新テスト
   `ur_34_a_release_gives_back_a_claim_when_loss_arrives_after_the_claim` はここで loss を publish し、
   `holder == Free`、close が呼ばれないこと、`set` が変わらないこと、Drop の panic が無いことを確かめる。
4. **遷移表の抜け（minor）**：N3 に `may_call_from_none` の `Free`・`Owner`・`Release`・`FromNone` と
   `FromNone` の二重取得を、N2 に `idle`・`active` なしでも holder が `Owner` なら `claim_release` が
   拒否されること（理由は holder）を足した。
5. **N8（minor）**：本物の `Claim` を外に持ち、`Claim { holder: Free }` の誤った返しだけを
   `catch_unwind` で捕まえる。panic の後に holder が変わっていないことを確かめ、最後に本物を返す。
6. **N9（minor）**：2 つの受信 stream に line を入れ、`book_move` で `seq = 7` の move を作る。
   同じ source で claim を返して取り直し、`Some(7)`・`Some(8)`・`None`、`FromNone`、`Release`、`Free` を
   比べる。private な field は直接変えない（`book_move` と遷移の関数だけ）。
7. **mutation 行**：新しい行は足さず、`busy`・`busy_seq`・`reading` を名指す 14 行を
   `holder`・`reading` と遷移の関数に当て直す（P252、P253、P382–P386、P389、P392、P398、P399、P404、
   P455、P496）。J279/J295/J312（host の participant）と P505/P512（変更なし）は触らない。

未決 A–G の解決（実装のとおり）：

- **A**：`end_read` は `reading = false` のとき、`give_back` は holder が claim と違うとき、
  production では状態を変えずに返る。test build だけ panic。N6/N8 が固定する。
  `give_back` は panic の前に `mem::forget` するので、二重 panic にならない。
- **B**：`holder` は production の guard に、`reading` は `await_reading` の待ちの判定と
  `read_reports_before_close` の観測に production で要る。両方 `pub(crate)` の accessor とし、
  `#[cfg(test)]` は付けない。field は private のまま。
- **C**：P385 は残す。booking の前に返す道は `Option<Claim>::take` で表現でき、
  `cold_booking` の probe テストが殺す。
- **D**：claim の後の loss は `release_claimed` probe で決定的に踏む（3）。
- **E**：S2 は lock の回数ではなく `refused_booking` probe の holder と `try_lock` で固定する（2）。
- **F**：`read_reports_before_close` は `begin_read` が false のとき読みを skip する。
  `tx.pass` の `plan_section` の false 分岐は残し、`end_read` を呼ぶ。
- **G**：`tx.pass` の `begin_read` が false なら device を呼ばず `false` を返す。

この修正の後、`test-plan.md` の「この段階では実行しない」は実装段階 1b-6a の指示で解除された。なお、上の §6「していないこと」は計画段階の記録であり、現在の状態は `report.md` の「## 実装」にある。
