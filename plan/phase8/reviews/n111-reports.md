# #111 + #119 — implementer reports (Sonnet supervising Go DeepSeek)

## Implementation (stages a–e)

Issues #111 and #119 are implemented in `~/local_work/Ez-SDRv4-n111` (branch `work/n111`, base `c31a2138`), uncommitted, in stages (a) to (e) with no send-backs. The gate checks are green, except one hang-signal test that fails only when run in the background. It passes in the foreground (details under Checks).

The cancel and the time reads no longer run under the admission or machine lock. Cleanup reads the Stopping and CleanedUp clocks before taking the machine lock. RS-6 step 1 now sets `frozen` and clears the queues under the admission lock, releases it, then takes and cancels the handles under a new `freeze_gate`.

## `git diff --stat`
13 files changed, 2379 insertions(+), 142 deletions(-).
- Production code, `crates/ezsdr-kernel/src/coordinator/`: `admission.rs`, `ending.rs`, `mod.rs`, `pipeline.rs`, `state.rs`, `stepping.rs`.
- Tests: `coordinator.rs`, `device_paced.rs`, `support/paced.rs`, `support/start_window.rs`.
- Specs: `design/04-run-and-session.md`, `design/06-kernel-coordinator.md`.
- Rows: `plan/phase7/tools/mutations.json`.

## Per stage
- **(a) Lock helpers and records.** A per-thread record of the admission, machine and freeze locks (`lock_record`). Every lock goes through `lock_admission`, `lock_machine`, `lock_freeze` or `try_lock_freeze`. The test doubles record `held` on each call. The #103 test is migrated, and T15 is added.
- **(b) #119.** Cleanup reads `now()` and the host clock's UTC time before the machine lock, then publishes the transition. T7 and T8 are added.
- **(c) `freeze_gate` and FreezeDispatch.**
  - The freeze runs in the order set `frozen` → release the lock → take wake and scheduled handles → cancel (under the gate).
  - The early withdrawal of the `advance_to` horizon takes the gate with `try_lock` and does nothing if a freeze holds it.
  - `dispatch_agenda` breaks out when dispatch is already frozen.
  - Tests T1, T2a/b, T3 to T6, T9, T10a to T10d.
- **(d) Sweep.** One `sweep()` helper plus `Phases` in `support/start_window.rs`, used by T11 to T14.
- **(e) Specs.**
  - RS-6 step 1, KC-20, KC-24a (rule, Ceiling, Tests), KC-29b, KC-43 and KC-46a are edited in place.
  - There are two Changes rows, one in each spec.

## Tests, rows and outcomes
Every row is killed. Rows were run through `simq` and `remote_mutate`; the logs are in `rows-{a,b,c1,c2,c3,d,e}.log` and `rows.log` in the task folder.

| Test | Row(s) | Outcome |
|---|---|---|
| T15 `every_admission_and_machine_lock_goes_through_the_recording_helpers` | N111-19, N111-1 | killed |
| T7 `cleanup_reads_the_stopping_and_cleaned_up_clocks_outside_the_machine_lock` | N111-8 and others | killed |
| T8 `an_abort_in_the_unlocked_cleanup_window_escalates_the_remaining_steps` | N111-9 and others | killed |
| T1 `freeze_runs_cancels_outside_the_admission_lock` | N111-2, N111-3 | killed |
| T3 `a_provider_holding_what_cancel_needs_while_it_submits_does_not_deadlock` | N111-4 | killed |
| T4 `freeze_takes_the_wake_handle_with_the_admission_lock_free` | N111-7 | killed |
| T5 `a_handle_fired_before_its_cancel_is_not_a_failure` | N111-10 | killed |
| T6 `overlapping_freezes_serialise_and_cancel_before_the_single_stop` | N111-11, N111-12 | killed |
| T2a `freeze_cancels_the_wake_handle_exactly_once`, T2b `a_wake_issued_after_the_freeze_is_not_cancelled` | N111-6 | killed |
| T9 `an_agenda_item_reached_after_the_freeze_is_not_admitted_and_requests_nothing` | N111-15 | killed |
| T10a, T10b | N111-13, N111-16 | killed |
| T10c `a_withdrawal_cancel_precedes_the_provider_stop` | N111-21 | killed |
| T10d `an_early_withdrawal_skips_when_a_freeze_holds_the_gate` | N111-22 | killed (bound) |
| T11 `sweep_an_orderly_run…` | N111-17, N111-18 | killed |
| T12 `sweep_an_abort_run…` | also kills N103-11 | killed |
| T13 and T14 (device loss, paced orderly finish) | none of their own | pass; they cover the data thread's reads and the paced path |

N111-22 is declared `"kill": "bound"` because a wait-based mutant cannot show up as an assertion. N111-14 appears in the Changes narrative only and has no row of its own. There is no N111-20, because the same mutant is the existing N103-11, killed by T7 and T12. I also ran the rows re-anchored or inside edited hunks: G29, C10, N103-1/2/3/5/6/7/10/11, 10 of 10 killed. The overlap was computed by `anchor.find` against the diff hunks, for phase7 only. No other list's rows overlapped an edited hunk.

## Checks
- **fmt and rules:** `cargo fmt --all --check` is clean. `rules.py --base c31a2138` reports 0 problems.
- **clippy:** `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- **Tests:** `cargo test --workspace --no-fail-fast` gives 3187 passed and 1 failed.
- **The one failure:** `t200_bare_uhd_script_judges_a_run_and_bounds_it` fails when cargo runs in the background (SIGINT, SIGQUIT or SIGHUP ignored, which looks inherited from the launch). It passes in the foreground (1 passed), so I count it environmental. This is inference; I did not run it at the base commit.
- **20× repeats:** `kc_24a` filter, 20 runs on each binary. `coordinator` runs 12 tests per run (T1, T3, T5, T7, T8, T11, T12, T15, the migrated #103 test and others) and `device_paced` runs 16. All 40 runs pass. That includes T2a/b, T4, T6, T9, T10a to d, T13 and T14. DeepSeek's own 20× runs of T11 to T14 also passed.

## Adversarial review, one line per test
- **T1, T3, T5:** the cancel hook records `held`. Moving the cancel back under the lock fails on "held.admission" or on the deadlock path; T3's timeout is reported as infra.
- **T2a, T2b:** one `Cancel` of the wake handle, and none for a wake issued after the freeze. A second freeze pass or a post-freeze cancel breaks the count.
- **T4:** the wake handle is taken with the lock free. Taking it under the lock shows in `FreezeReading.held`.
- **T6:** serialisation plus cancel-before-stop, checked through `Serialise`/`Serialised` lines against `radio:stop`. Without the gate, the second `Serialised` precedes the first cancel.
- **T7, T8:** `Now`/`Utc` outside the machine lock, in order. DeepSeek added a whole-run rule after a per-pair check let N111-8 survive. T8's abort in the unlocked window escalates the remaining steps.
- **T9:** a frozen check moved before the lock would still pass T9. N111-15 kills the missing check. This is recorded for the Sol code review.
- **T10a, T10b, T10c:** the withdrawal's cancel is under the gate, once, before the stop. N111-16 and N111-21 break each case.
- **T10d:** the skip path is checked by a wait-based mutant (`bound`), not by a reading.
- **T11, T12:** the sweep fails on `held.admission || held.machine` for any call. N111-17 and N111-18 reached it in the start batch and in a Module submission, as T11's failure text shows.
- **T13, T14:** the data thread's reads are covered, and cancels fall in the Running and Stopping windows as required.

## Trial record
- **Runs:** DeepSeek ran 7 stages, and I made 0 send-backs. Stage (e) was started twice by mistake (a launch I thought had failed); both wrote the same text. I audited it verbatim.
- **What I wrote into the tasks:** the production lines for stages (b) and (c), the `freeze_gate` design, the order of `FreezeDispatch`, the withdrawal's `try_lock` and `dispatch_agenda`'s frozen check. The tasks also named every test, row and spec edit.
- **My own edits:**
  - `design/06` KC-29b: I rephrased the withdrawal sentence so "so no later round runs…" stays attached to the cancel.
  - Unit test `held_records_the_calling_threads_guards`: I removed its `thread::spawn`, because governance `po_11` forbids a spawn under `coordinator/`. The per-thread property is observed by the Call-based tests instead.

## Deviations from plan v3
- **T11, T12:** the Provider's line is `p:update:test.gain=1.0` (there is no `p:took`). `x:submit:ok:2` comes from the Executor's own step. The test thread submits once more, which returns id 3 and stays in `undelivered`.
- **T11, T12:** the manual clock advances to 500 before `finish`, so the last two `Now` samples are not all 0.
- **T13, T14:** a `FakeHostClock` goes under `WatchedClock`.
- **Specs:**
  - RS-6 is written in place at the existing step-1 text.
  - KC-24a's rule and `*Test:*` line are split over two lines so that `rules.py` accepts the line count.
  - The Ceiling is the new text from the amendments.

## INFERRED or open
- **Not covered:** `far_future_stop`'s threshold is borderline for T6's offset. `freeze_trace` assumes the first freeze performance is the data thread's in the c3 fixtures. T7's `Now` sample equality is vacuous for early transitions, which the helper doc says.
- **For Sol:** Opus n1 notes that no row pins "removal under the gate". Opus n2 is the T9 frozen-check placement above.
- **Not run by me:** the full mutation lists (they belong to the gate) and `gate.py`. No commit or push.

Ponytail: I did not recheck the vision parts or the other accepted specs beyond DeepSeek's grep of "admission lock". The signal-trap failure is judged environmental from one foreground pass, not verified against the base commit.

## Fix round 1 (after the Opus + Terra sub-step review)

修正ラウンド1を適用しました。`fmt`・`clippy`・`rules.py` は通っています。`cargo test --workspace` は 1 本落ちましたが、負荷によるもので、今回の変更とは無関係です。

**`git diff --stat`**(7 ファイル、+169/−157)

| ファイル | 変更 |
|---|---|
| `coordinator/mod.rs` | `Point::BeforeWithdrawalGate`(testing 専用)を追加。`Reading.admission_held` の doc を 1 行追加(Opus 6) |
| `coordinator/stepping.rs` | withdrawal の `try_lock_freeze` 直前に `reach` を 1 つ追加。本番の振る舞いは変わりません |
| `tests/device_paced.rs` | −268/+120 程度。`tests/support/start_window.rs`: `Hooks.on_cancel_return` を追加 |
| `design/04`・`design/06` | spec 文面(下の表)|
| `plan/phase7/tools/mutations.json` | N111-22 から `"kill":"bound"` を削除。N103-10/11 の名前変更。N111-23/24/25 を追加 |

**指摘ごとの修正・テスト・行**

行の結果はすべて `remote_mutate`(phase7、sim02/sim03)で 12/12 killed です(`rows-fix1.log`)。N111-3/6/11/12/13/15/16/21/22/23/24/25 を流しました。割り当てた assertion の下の行に出ている panic 文面は、/tmp の probe コピーで読んだものです。

| 指摘 | 修正とテスト | 行と結果 |
|---|---|---|
| Terra1 + Opus7(T9) | 新しい行を追加。T9 は変更なし。`frozen` 読みを park 点より前へ移すと、freeze が park 中に完了して KC-17 の Abort に上がる | N111-24 は `also` 空の assertion で死にます(`KC-20: a skipped agenda item is no failure: [Failed ...KC-17...]`)|
| Terra2 + Opus4(T10a/T10c/T6) | T10a の park を新しい `BeforeWithdrawalGate` に移しました。cancel の戻りを `Hooks.on_cancel_return` で記録し、T6 と T10c の順序を戻りで assert します | N111-25(除去をゲートより前へ)は `cancels[0].thread == data` で死にます。N111-21 は `held.freeze` の assertion で死にます |
| Terra3 + Opus3(T10d) | 10 s 待って見えなければ、infra panic にせず `gate_c` を開け、最後の `!withdrawn:parked` で判定します | N111-22 は `bound` なしで assertion kill です(`the withdrawal did not reach Withdrawn`)|
| Opus2(T4) | `BeforeDrop` を待ってから W_s を起動し、ゲートが閉じている間に refusal が返ることを assert します | N111-23(take のみを drop より前へ)は `submission waited for the wake's schedule` で死にます。N111-2 は T1 のまま残しました |
| Opus1・9・10(spec 文面) | KC-29b の Test 行に T10a–d を追記。Changes 行に KC-2 と KC-29b を追加。KC-2 の device-paced 列挙から KC-24a を削除。RS-6 の step 1 を「refuse every further submission (KC-24 step 1: dispatch is frozen) …」に変更 | — |
| Opus5・8(整理と名前) | `start_horizon_run` と `entered` を追加(下の「DeepSeek の作業」を参照)。N103-10/11 の名前を変更 | — |

**DeepSeek の作業**
- 範囲は T10a–d と T2a/T2b の 6 本のヘルパ化です。diff を読み、外側の冗長な `.map` を 1 つ消しました。
- T9 は spec をカスタマイズしているので、ヘルパ化しませんでした。T6 は freeze の readings を溜めるので `freeze_trace` が合わず、そのままにしました。

**adversarial の 1 行メモ**
- T9: 到達は `agenda:parked` と `freeze:Done` の待ちで保証されます。最悪の deschedule は W_c が park 中の場合で、test は待つだけです。
- T4: W_s が 10 s ずっと deschedule されると、正しいコードでも assertion で落ちます(コメントに記載)。
- T10d: 遅いホストでは通ります。落ちるのは withdrawal がゲートを取った場合だけです。
- T10a: 凍結した後に W_c が除去すると、N111-25 の経路になります。park 点の位置がそれを拾います。
- T6・T10c: 順序の assertion は cancel の戻りの行で見ています。

**チェック**
- `cargo fmt --all --check` は OK。`cargo clippy --workspace --all-targets -- -D warnings` は OK。
- `rules.py --base c31a2138` は 0 問題。
- `kc_24a` の 16 本を ×20 回して全回 pass。`cargo test --workspace --no-fail-fast` は 3187 pass、1 fail。
- `t200_bare_uhd_script_judges_a_run_and_bounds_it` は workspace 実行の foreground で ok。
- 落ちた 1 本は `n47s5_front_rehearsal_on_the_fake` で、host lateness(`TIME_ERROR late`)が原因です。単独では 67 s で pass します。UHD crate の wall-clock 依存で、今回の diff とは無関係です。

**差異と INFERRED**
- 「check と lock の間」の窓は作っていません。N111-24 は `reach` より前へ移した変異だけを殺します。`reach` と lock の間に挟んだ変異には窓を作れないので、残ります(INFERRED、test 内にコメント済み)。
- `full.log` に `phase7/G09` の不合格が 1 件あります。今回は触れていません。

**スキップした点**
- coordinator.rs の SimAuthority セットアップの共通化はしていません。
- T15 の `begin_stopping(` スキャン拡張(Opus8 の任意案)もしていません。

主なファイルのパス
- `~/local_work/Ez-SDRv4-n111/crates/ezsdr-kernel/tests/device_paced.rs`
- `~/local_work/Ez-SDRv4-n111/plan/phase7/tools/mutations.json`
- `~/.cache/ezsdr-fixes/2026-11-19-n111/rows-fix1.log`

## Fix round 2 (after Sol's code review)

F1–F4 を直しました。全 N111 行と C06・G10・G29 が殺せています。fix round 2 の最終ツリーには rows 再実行も `cargo test --workspace` の全緑もありません（理由は「検査結果」に）。`/tmp` に n111 のログ以外の残りが7件ありますが、DeepSeek のコピーかは未確認です。worktree は `~/local_work/Ez-SDRv4-n111`、commit はしていません。

## F1 の設計メモ（production 変更の前に決めたもの）
- **仕組み**
  - データスレッドの early stop (`stop_early`) が freeze gate を 1 回だけ取り、freeze から最後の Provider stop まで保持する。
  - gate を取ったとき `frozen` が既に立っていれば（cleanup の freeze が先に走った場合）、freeze は行わず stop だけを行う。
  - cleanup 側の `StopProviders` は gate を取らない。
- **本体の分割**: `Ops::freeze_held()` を新設し、freeze 本体をそこへ移した。cleanup の FreezeDispatch は `lock_freeze()` のあと `freeze_held()` を呼ぶ。
- **ロック順序**
  - freeze_gate → admission、freeze_gate → wake_handle / scheduled、freeze_gate → done → Provider slot（stop）。
  - gate を取る箇所は、cleanup の FreezeDispatch、`stop_early`、出撤回の `try_lock` の 3 つ。どれも他のロックを持たずに取る。
  - 逆順で取る経路は grep で見つからなかった。
- **stop と cancel の 6 組**

| 組 | 排除の仕方 |
|---|---|
| データ stop × データ freeze | 同一スレッドで順番に実行 |
| データ stop × cleanup freeze（Sol の経路） | gate で排除。cleanup の cancel は stop の後になる |
| データ stop × 出撤回 | 出撤回の cancel は gate 下なので排除 |
| cleanup stop × cleanup freeze | 同じ cleanup スレッドで順番に実行。RS-8a の例外あり |
| cleanup stop × 出撤回 | 出撤回は cleanup より前に完了する（呼び出しと cleanup は同一スレッドで順番に実行される） |
| cleanup stop × データ freeze（遅れて走る場合） | `frozen` の確認でスキップするので排除 |

- **deadlock / hang の評価**
  - gate を取る側はすべて他ロックを持たない。
  - cleanup の stop が gate を待つ経路を作らなかったので、cancel が止まっても Provider の stop は必ず呼ばれる。
  - データスレッド上の Provider::stop が止まると、cleanup の freeze は RS-8a の 5 秒で放棄され、cancel が行われないまま残る。cleanup 全体が 5 秒を超えて止まる経路は作っていない。
  - 本件と無関係に、データスレッドの `stop` が戻らなければ step 3 の join も止まる（既存の挙動）。
  - この「stop が止まると freeze が放棄される」点を design/06 KC-24a の Ceiling に明記した。

## findings ごとの結果

**F1: Provider stop と遅れた cancel の重なり**
- **修正**: 上記のとおり。Sol の経路を再現するテスト `kc_24a_the_data_threads_provider_stop_does_not_overlap_a_late_horizon_cancel` を先に書き、修正前の木で assertion (a) の失敗を確認した。`cancel:entered`、`radio:stop`、`cancel:returned` の順になる。
- **追加テスト**: `kc_24a_an_early_stop_after_the_cleanups_freeze_performs_no_second_freeze`。
- **test hook**: `FreezePoint::BeforeStop` を、Provider stop 直前（fragment ごと）に置いた。
- **rows**
  - N111-26（stop の前に gate を放す）、N111-27（`if true` にして freeze を再実行）、N111-28（データスレッドの gate 取得を外す）を追加した。いずれも assertion で殺せた。
  - N111-26 は最初の hook 位置では生き残った。hook を `drop(gate)` より後ろ（loop 内）へ移して直した。
  - N111-12 は廃止した。freeze 本体が gate を持たなくなり、gate を drop する mutant が書けなくなったため。

**F2: N111-22 / N111-23 が 10 秒の timeout で死んでいた件**
- 各 hook を、その操作を行う helper の中へ移した。
  - `Serialise` は `Shared::lock_freeze` の中へ。
  - `BeforeWakeTake` は `Ops::take_wake_handle` の中へ。
- N111-23 の killer は、`BeforeWakeTake` の reading で admission が解放済みかを見る形にした。mutant は約 0 秒で assertion に落ちる。
- N111-22 の killer は、出撤回スレッド自身の `freeze:Serialise` 行が無いことを見る形にした。mutant は約 0 秒で assertion に落ちる。
- 副作用で survive / ERROR になった rows を直した。
  - N111-16: 「データスレッドの stop 後に gate が空く」前提が崩れたので、Authority 経由の `wake:after_stop` を待つ形に直した。
  - N111-11 と N111-28: hook が lock と一緒に消えて infra ERROR になったので、mutant 側に hook を残した。
- N111-2 は N111-23 と同一になったため廃止した。N111-6 は anchor を付け直した。

**F3: T9 の `frozen` 読み位置**
- hook と lock は元から隣り合っていて、その間に test が割り込める窓がありません。間に読みを入れる mutant は、どの test からも見分けられません。
- 決定どおり fallback を採り、残余をテストのコメントに明記しました。この mutant 用の row は作っていません。

**F4: 生の mutex の直接アクセス**
- admission / machine / freeze gate を `Serial<T>` 型に包み、mutex を state.rs 内に private にした。
- probe 用に `Serial::is_held`（テスト時のみ）を用意した。
- alias 経由のロックは、型の不一致と private field のエラーでコンパイルが通らないことを /tmp のコピーで確認した。
- これに伴い、ソース走査テスト `kc_24a_every_admission_and_machine_lock_goes_through_the_recording_helpers` と N111-19 を廃止した。C06 は `lock_machine()` 経由に付け直した。
- design/06 の Test 行から走査テストの名前を外した。

**spec**
- design/04 RS-6、design/06 KC-24a と KC-46b の本文、Test 行、Changes 表を更新した。
- `rules.py` は 0 problem。

## 新規・変更テストの敵対的確認
| テスト | 確認した mutant と結果 |
|---|---|
| F1 テスト（`…provider_stop_does_not_overlap…`） | N111-26 mutant で assertion (a) が失敗した。`cancel:entered` と `held.freeze` の読みで状態に到達していることを確認した |
| T12（`…performs_no_second_freeze`） | `if true` mutant で「freeze が 2 回走った」という assertion が失敗した。データスレッドを `Serialise` で止めてから cleanup の freeze 完了を待つ順序にしてある |
| T4（`…takes_the_wake_handle…`） | take を drop の前へ動かすと `held.admission` の assertion で失敗した。W_s は `BeforeWakeTake` の読みが届いた後に起動する |
| T10d（`…skips_when_a_freeze_holds_the_gate`） | `lock_freeze()` へ変える mutant で「出撤回が gate を待った」という assertion が失敗した |
| `…does_not_cancel_a_handle_the_freeze_took` | cancel 無条件化 mutant で cancel 数の assertion が失敗した（`cancels.len()` が 2、期待は 1） |

6 テストとも 20 回連続で通りました。各スレッドの最悪の descheduling 点は、各テストのコメントに書いてあります。

## 検査結果
- `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`rules.py --base c31a2138` は通っています。
- rows は 2 回に分けて走らせ、全て killed です。
  - 1 回目は N111-* 全体と C06・G10・G29 の 26 本で、N111-11・16・28 の 3 本が失敗しました。
  - 修正後、その 3 本だけ再実行して全て killed でした。
  - 1 回目に通った 23 本は、そのあとの変更（N111-16 のテスト修正、N111-11・28 の row 付け替え）では再実行していません。
- `t200_bare_uhd_script_judges_a_run_and_bounds_it` は ok です。workspace 実行内の結果と、その test 名だけを指定して走らせた実行の両方で確認しました。
- `cargo test --workspace` の最終ツリー全緑は未確認です。
  - 最終ツリーでの 3 回目の実行（`--no-fail-fast`）では、`uhd_59_a_loss_after_an_overrun_stops_the_run_and_keeps_the_tail` が 1 件落ちました。
  - 他の 73 binary は ok です。
  - 失敗メッセージは `EZSDR-TIMING`（T0 が 4.7M ticks ずれ、許容は 2M）で、負荷平均が 21 前後の時間帯でした。
  - 同じテストを単独（`--exact --test-threads=1`）で 1 回、`uhd` binary 全体を負荷の下がった状態で 3 回（各 36 件）走らせ、全て通りました。
  - 最初の workspace 実行（`EXIT 0`）はテスト編集の前のものです。
  - 負荷によるタイミング flake と見ていますが、断定ではありません。
- 差分の規模は `git diff --stat` で 10 files、+421 / -190 です。
  - 内訳は `ending.rs`、`mod.rs`、`paced.rs`、`pipeline.rs`、`state.rs`、`tests/coordinator.rs`、`tests/device_paced.rs`、design/04、design/06、`mutations.json`。

## 逸脱
- 指示にない C06 の付け直しを 2 回行いました（freeze 分割のとき、`Serial` 化のとき）。
- N111-2・12・19 の 3 行を廃止しました。いずれも同一化または mutant が書けなくなったためです。
- design/ の spec 本文は DeepSeek ではなく私が直接編集しました。

## INFERRED と残余
- 次の 3 点は、証明も実行テストもありません。
  - 最後の freeze より後に公開された handle が stop の後に cancel される経路は、無いと証明できません。本文には「stop 開始時に cancel が走っていない」ことだけを書きました。
  - データスレッド上の Provider::stop が詰まると、cleanup freeze の cancel が行われないまま残ります。
  - UHD の stop は約 3 秒なので、5 秒の deadline に収まると見ています。
- 該当する Provider::stop が詰まるテストは device-paced には無く、grep（`wedg`、`never return`）で見つかったのは Simulation の cleanup 側だけです。

## 関連ファイル
- `~/local_work/Ez-SDRv4-n111/crates/ezsdr-kernel/src/coordinator/paced.rs`
- `~/local_work/Ez-SDRv4-n111/crates/ezsdr-kernel/src/coordinator/ending.rs`
- `~/local_work/Ez-SDRv4-n111/crates/ezsdr-kernel/src/coordinator/state.rs`
- `~/local_work/Ez-SDRv4-n111/crates/ezsdr-kernel/tests/device_paced.rs`
- `~/local_work/Ez-SDRv4-n111/plan/phase7/tools/mutations.json`
- `~/local_work/Ez-SDRv4-n111/design/04-run-and-session.md`
- `~/local_work/Ez-SDRv4-n111/design/06-kernel-coordinator.md`
- ログは `~/.cache/ezsdr-fixes/2026-11-19-n111/` の `dsg-1..5.log`、`rows-fix2.log`、`rows-fix2b.log`、`ws-fix2d.log`

確認していないこと: gate 保持中に長時間かかる Provider::stop の実機での挙動。
