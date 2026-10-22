STATUS: done

# 1b-6b 報告：UHD Module の段階を遷移の関数だけが変える

この文書の前半は計画段階（2026-10-22）の記録で、現在の tree の状態ではない。実装の記録は末尾の「実装」。

## 何をしたか

- 指示書 `impl-brief.md` と設計メモ `plan/phase8/35-uhd-stream-state.md`（R2）を読んだ。
- HEAD `2af9afd` の `crates/ezsdr-radio-uhd/src/provider/{core,rx,tx,control}.rs`、`tests/fake.rs`、spec 18 の UR-17・UR-25、`plan/maintenance/tools/mutations.json` を読んだ。
- 計画段階で `test-plan.md` を書いた。コード・テスト・mutation 行はその後「実装」で書いた。
- 計画段階の終了時はリポジトリは `2af9afd` のまま、tree はきれいだった。実装後は 6 ファイルが modified（末尾）。`cargo fmt`・`rustfmt`・`git add`・`commit`・`push`・`stash` は使っていない。背景の job は起こしていない。

## 計画の要点（`test-plan.md`）

- 型：`Phase { Idle, Pending, Active { segment: Option<Segment>, delivered: Option<(i64, Config, i64)> } }`（`Segment = (i64, Config, Option<u64>)`）。field は private。`Default` は `Pending`（`Core::new` の start 前の `idle = false` を保つ）。メモの `Option<i64>` は採らず HEAD の tuple を残した。
- 遷移は `Streams` の 6 つだけ：`starting`（`Control::new`）、`search`（`Rx::begin`）、`started`（`Rx::new`・`begin` 成功）、`delivered`（`Rx::samples`）、`finished`（`Rx::finish`・`Tx::do_switch`、旧 segment を返す）、`plan_read`（`Tx::plan_section`）。読む側は `phase`・`is_idle`・`active`・`first_undelivered`。
- 系列 P1–P11 のそれぞれに、今あるテスト（名前・assert の内容）、新しいテスト／既存への追加、観測方法、落とす誤実装を書いた。
- 新しいテストは thread を使わない 8 本：N1 `ur_25_a_stream_starts_idle_or_waiting`、N2 `ur_26_a_receive_start_without_a_segment_stays_active`、N3 `ur_26_a_receive_start_of_no_planned_segment`、N4 `ur_25_a_search_leaves_the_stream_pending_or_idle`、N5 `ur_17_a_receive_segment_moves_from_pending_to_active_to_pending`、N6 `ur_25_a_transmit_read_sets_idle_pending_or_active`、N7 `ur_26_a_finished_stream_clears_only_its_own_floor`、N8 `ur_25_a_refused_last_transmit_change_ends_idle`。既存の thread テスト（P2–P7、P11 の fake）は形を変えず、phase の assert を足す。
- 印の直接読み書き：production 19 行、test 54 行（`Item.delivered`・`manifest.events.delivered` を除く）。test の全位置と置き換えを `core.rs`・`rx.rs`・`tx.rs`・`control.rs` ごとに表にした。`idle = true` は `search(k, false)`、`idle = false` は `search(k, true)`、`delivered` の書きは `started` + `delivered`、読みは `first_undelivered`。
- mutation：今の行のうち R2 が触るのは J46・J54・P258・P404・P445・P470・P472・P382。J46/J54 は `book` の local 名と filter 行を残すのでそのまま、P258/P404/P445/P382 は old/new を新しい呼び出しに当て直し、P470/P472 は `Streams::finished`（`core.rs`）の本体へ移して P472 の殺し手を N7 にする。触らない行（J11・P252/P253/P496・P357・P503・他 crate）も一覧にした。新しい行はコードと一緒に足す。

## 食い違い（メモ R2 と HEAD）

1. TX の初期化：メモは clock があれば `Active`、HEAD の `Control::new` は `Pending`（最初の `plan_section` で `Active`）。HEAD を採る。観測は変わらない。N1 が固定。
2. RX の初期化は HEAD では 2 段（`Control::new` が Idle/Pending、`Rx::new` が Active）。`Provider::start` は RX を 1 本ずつ作り、そのたびに spawn するので、その間に別の owner が T0 前の loss を publish できる。2 段を写し、N3 がその間の loss を実際に作る。
3. メモの `delivered: Option<i64>` は HEAD の `(origin, config, k)` を落とす。HEAD の tuple をそのまま残し、`first_undelivered` は tuple を返す。`segment: None` は T0 の segment が start の途中の loss で消えた場合に到達し、HEAD は floor の tuple を保つ。End が適用するかは `cuts` が `line.made(...).cut.is_none()` で決めるので、消えた segment では適用されない。N2・N3・J54/P9 が固定。
4. `plan_section` で clock があり `plan.get(at)` が None のとき、メモの文言は「今の segment で Active」だが HEAD は `Active { segment: None }`。HEAD を採り、N6 がこの枝を固定する。

## リスク

- N1–N7 は phase の読みが主で device call を見ない。遷移の配線は P2–P7 の既存テストが受け持つ。N シリーズだけで配線が正しいとは主張しない。
- `phase()` を `pub(crate)` にして test から観測する。production の読みが `is_idle`・`active`・`first_undelivered` に限られることを review で grep する。
- P472 の殺し手 N7 は `Streams` の unit なので、`Rx::finish` が `finished` を呼ぶ配線は見ない。配線は P470 のテストと P11 の fake テストが見る。
- 実装後の確認は `cargo test -p ezsdr-radio-uhd --lib` と `--test fake`、mutation は `remote_mutate.py`（fix round は触った行だけ、gate で全リスト）。計画段階では実行していない。実装後の結果は末尾。

## 実装

### 計画の修正（Sol の計画レビュー）

前段階で `test-plan.md` とコードの一部がすでに書かれていた（順序が逆）。この段階で `test-plan.md` を直して末尾に「## 修正（Sol の計画レビュー）」を置き、残り（P6/P7 の assert、N8、spec 18、mutation 行）をその後に実装した。

1. P6：既存の `ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch` の最後を 3 個目の segment の `Active`（`planned[3]`）として検査し、switch 元が `completed` に載ることも見る。別に N8 `ur_25_a_refused_last_transmit_change_ends_idle` を足し、後続の無い拒否で旧 segment が完了し、clock が無く、最後が `Idle` になることを見る。
2. P4：拒否からの再開後の `active` を `by: Some(2)`、rate 100、floor 0 で検査した。configure 中の loss の既存テストに、戻った直後が `Pending`、active も floor も無いことを足した。
3. P1b：N3 `ur_26_a_receive_start_of_no_planned_segment` は T0 の line を作り、T0 前の loss を実際に book して segment を消してから `Rx::new`・`finish` を通す。phase、完了記録なし、loss の `End` と cut が変わらないことを見る。`Provider::start` が RX を 1 本ずつ spawn する事実に合わせて計画の thread の説明も直した。
4. `delivered` は HEAD の tuple `(origin, Config, k)` を残した。`Active { segment: Option<Segment>, delivered: Option<(i64, Config, i64)> }`。メモの `Option<i64>` は採らなかった。
5. N7 は Abort の `End` を root tick 800 000（sample 4 000、stream 0 の 3 000 より後）に置き、stream 0 が instant ではなく自分の floor で切れることを見る。
6. N4 は `Active` から `search(false)` を 1 回、N5 は `Pending` 中に `delivered` を 1 回呼び、それぞれ前の状態から不変を確かめる。
7. `Rx::finish` の配線を殺す P559 を足した。N6 は clock があり plan に segment が無い枝も検査する。

### 変えた本体

| file | 行数（+/-） | 内容 |
|---|---|---|
| `crates/ezsdr-radio-uhd/src/provider/core.rs` | 266 / 29 | `Segment` と private `Phase`、`Default = Pending`、遷移 6（`starting`・`search`・`started`・`delivered`・`finished`・`plan_read`）、読み 4（`phase`・`is_idle`・`active`・`first_undelivered`）。`book` は `first_undelivered`、release の判断は `is_idle`/`active` に。`StreamState` の `idle`・`active`・`delivered` を削除。 |
| `crates/ezsdr-radio-uhd/src/provider/rx.rs` | 92 / 17 | `Rx::new` は T0 の stream があるとき `started`、`begin` は `search`、開始成功は `started`、`samples` は `delivered`、`finish` は `finished` の戻り値を `complete` へ。 |
| `crates/ezsdr-radio-uhd/src/provider/tx.rs` | 131 / 10 | `plan_section` は `plan_read`、`do_switch` は `finished` の戻り値を `complete` へ。 |
| `crates/ezsdr-radio-uhd/src/provider/control.rs` | 37 / 33 | `Control::new` は `starting`（RX は T0 の stream、TX は clock の有無）。from-none は `is_idle` を読む。 |
| `design/18-uhd-radio.md` | 9 / 6 | UR-17・UR-25 の文と `*Test:*`、UR-26 の `*Test:*`、Changes の 1 行。 |
| `plan/maintenance/tools/mutations.json` | 148 / 58 | 6 行の当て直しと 9 行の追加。 |

HEAD と構文上違うが production で同値な点：`finished` の `Pending` は HEAD の「active なし・idle false」と同じ（`Rx::finish` は `self.stream` が None なら早期 return するので、Idle のまま終わる stream の phase を動かさない）。`search(false)` は HEAD なら残り得た `active` も消すが、`begin` は stream が無いときだけ走るので到達不能（N4 が固定）。`Rx::new` は stream が None のとき `started` を呼ばないが、`Control::new` の `starting(false)` がすでに `Idle` にしている。`plan_read` の clock あり・segment なしは HEAD と同じ `Active { None }`。

### テスト

新規 8 本（すべて thread を使わない）：

- N1 `ur_25_a_stream_starts_idle_or_waiting`（`core.rs`）
- N2 `ur_26_a_receive_start_without_a_segment_stays_active`（`core.rs`）
- N3 `ur_26_a_receive_start_of_no_planned_segment`（`rx.rs`）
- N4 `ur_25_a_search_leaves_the_stream_pending_or_idle`（`core.rs`）
- N5 `ur_17_a_receive_segment_moves_from_pending_to_active_to_pending`（`core.rs`）
- N6 `ur_25_a_transmit_read_sets_idle_pending_or_active`（`tx.rs`）
- N7 `ur_26_a_finished_stream_clears_only_its_own_floor`（`core.rs`）
- N8 `ur_25_a_refused_last_transmit_change_ends_idle`（`tx.rs`）

変えた既存テスト：

- `core.rs`：`ur_34_a_release_skips_a_stream_held_busy_by_from_none`、`ur_34_a_release_closes_an_idle_streamer_needed_by_a_move`、`ur_34_a_release_keeps_an_idle_streamer_with_no_peer_need`、`ur_26_an_end_takes_delivered_from_each_segment_s_own_stream`、`ur_25_a_claim_release_needs_a_free_idle_stream_with_no_active_segment`、`ur_25_a_claim_from_none_admits_a_read_in_flight`、`ur_25_a_begin_read_needs_a_free_stream_and_no_read`、`ur_25_a_begin_shutdown_read_ignores_the_holder`、`ur_25_move_source_busy_reads_the_owner_of_the_move`、`ur_34_a_release_gives_back_a_claim_when_loss_arrives_after_the_claim`。setup の `idle`/`delivered`/`active` 書きを遷移に置き換え、claim_release は `Pending` と `Active` を別々に拒む形にした。
- `rx.rs`：`ur_29_a_loss_after_a_configure_starts_no_receive_segment`、`ur_26_a_receive_start_queued_by_the_stop_ends_its_owner_without_detaching`、`ur_26_a_stream_s_delivered_is_cleared_alone`、`ur_25_a_refused_segment_is_booked_by_uhd_rx`。phase / active / floor の assert を足した。
- `tx.rs`：`ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again`、`ur_25_a_refused_transmit_clock_is_not_adopted`（setup）、`ur_25_uhd_tx_reads_the_plan_again_when_it_changes`、`ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch`、`ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing`、`ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call`。
- `control.rs`：`idle` を直接書く helper・setup を `search` に置き換え、`ur_26_an_orderly_stop_cuts_at_its_instant` は `delivered` を `started` + `delivered` に、`ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing`・`ur_25_an_enable_behind_an_owner_s_open_is_a_cold_change` は `is_idle` にした。他は §4 の一覧どおり setup のみ。

`grep -rnE "\.(idle|active|delivered)\b" crates/ezsdr-radio-uhd` で、`Item.delivered` と method 名を除いて production・test とも field の直接アクセスは 0。`phase()` はテストの観測用で、production の読みは `is_idle`・`active`・`first_undelivered`。

### mutation

- 残した行：J46・J54（`book` の local 名 `delivered` と filter の行は変えていない）。
- 当て直した行：P258（`streams.delivered(...)` の引数）、P382（comment を phase の言葉に）、P404（`streams.search(key, true)`）、P445（`first_undelivered`）、P470（`Streams::finished` 本体へ、全 RX stream を消す変異）、P472（同本体へ、phase を消さない変異。filter は N7）。
- 足した行：P552（`starting`）、P553（`search`）、P554（`started` の floor）、P555（`delivered` の値）、P556（`finished` の置換先）、P557（`plan_read` の clock/switch）、P558（`first_undelivered`）、P559（`Rx::finish` の配線）、P560（`plan_read` の segment）。
- `idle`/`active`/`delivered` を old/new に持つ既存行と、当て直し・追加の行の 27 行を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:3,sim03:3 <IDs>` で流した。**27/27 killed。** ID は J11 J13 J23 J46 J54 P252 P253 P258 P357 P382 P404 P445 P470 P472 P489 P496 P503 P508 P552 P553 P554 P555 P556 P557 P558 P559 P560。
- `mutations.json` の全 900 行について、`old` が対象 source にちょうど 1 回一致することを確認した（0 行が不一致）。新しい filter の test 名はすべて実在する。テストの rename・削除はしていないので、消えた filter 名は無い。
- 全リストは brief のとおり orchestrator が流す。実装役は流していない。

### spec 18

- UR-17：floor を「active な phase が持つ、まだ届けていない最初の sample」と書き、`segment: None` でも tuple を保つこと、End の `cuts` 照合は変えないことを保った。`*Test:*` に `ur_17_a_receive_segment_moves_from_pending_to_active_to_pending` と `ur_26_a_receive_start_of_no_planned_segment` を足した。
- UR-25：Ownership に phase の定義を足した。`Idle` は始めるものが無いとき、`Pending` は segment/switch を始める前と segment を終えて次を見る前、`Active` は処理中。`segment` は plan に無いとき None、`delivered` は uhd-rx だけ。`idle mark` を読む/動かすという文は phase の言葉に直した（歴史的な Changes 行は残す）。`*Test:*` に N1・N4・N6・N8 を足した。
- UR-26：`*Test:*` に N7 を足した。
- Changes：2026-10-22 の 1 行を表の末尾に置き、decision record を `plan/phase8/35-uhd-stream-state.md` §2 R2 への link にした。
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` は 0 problems。

### 確かめたこと

- `timeout 300 cargo test -p ezsdr-radio-uhd`：lib **259 passed**、fake **264 passed**、hardware 0、uhd_api 0（feature なし）、doc 0。失敗なし。
- `timeout 300 cargo test --workspace --no-fail-fast`：exit 0、失敗・panic なし。
- `timeout 300 cargo clippy --workspace --all-targets -- -D warnings`：pass。
- docker `ezsdr-v4-dev:uhd4.10`：clippy（`--features uhd --all-targets -D warnings`）pass、`cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api` は lib **267 passed**、uhd_api **5 passed**。
- 決定性：同じ lib binary を `--test-threads=8` で 20 回、**20/20 回 259 passed、0 failed**。
- セルフレビュー：reviewer は実装役と同じモデル、1 回だけ。**P1 なし**。P2（UR-25 の phase 定義が `segment: None`・RX だけの `delivered`・`finish` 後の `Pending` を説明していない）と P3 3 件（N8 の cut に sample index でなく root tick を使っていた、N3 の SampleClock の node、Changes の record が実在しない link）をすべて採用して直した。P1 が無かったので 2 回目は流していない。
- `cargo fmt`・`rustfmt` は流していない。`git add`・commit・push・`stash` はしていない。

### 穴

- 全 mutation リストは流していない（27 行のみ。残りは orchestrator の gate）。
- phase の新しいテストは device call を直接見ない。P2–P7 の既存テストと fake の P11 が配線と device call を受け持つ。
- セルフレビューは同じモデル 1 回だけで、独立レビュー（Sol）は orchestrator の段階。
- `search(false)` が `Active` を消す差は production で到達不能だが、将来 `begin` の呼び出し条件が変わると挙動が変わる。N4 の 3 がその前提を記録している。
- `Rx::new` が stream None のとき phase に触らない点は `starting(false)` の Idle に依存する。今の `Control::new` → `Rx::new` の順でだけ正しい。
