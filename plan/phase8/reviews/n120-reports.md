# #120 + #121 — implementer reports (Sonnet supervising Go DeepSeek)

## Plan (phase A)

計画書を書きました。リポジトリには一切変更を加えていません（`git status` は空です）。

**計画書**: `~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md`
**DeepSeek の素の棚卸し**: `~/.cache/ezsdr-fixes/2026-11-20-n120/ds-1-result.md`

**棚卸し**
- 表は約 17 行で、coordinator の全ロックと、保持中に呼ぶ supplied 呼び出しを載せています。実行した grep は計画書 §1.1 に書いてあり、再実行できます。
- 移すのは 2 箇所です。
  - `wake_handle` は `schedule` をまたいで保持されています（#120）。
  - `links` は `drops()` をまたいで保持されています（#121）。
- 残すのは 3 箇所です。理由は表に書きました。
  - admission の下の registered check と `coerce`。
  - freeze gate の下の `cancel` と `Provider::stop`。
  - slot ロックの下の Module メソッド。
- 他のロックの下には supplied 呼び出しがありませんでした。
- DeepSeek の抜けと誤りを確認しました。
  - 抜け: `emit_device_lost` の `now()` が slot の下（`ending.rs:364`, `:431`）と、data thread の早期停止中は gate の下（`:399`）で呼ばれます。
  - 誤り: 「`links` と `link_drops` の両方を保持して `drops()` を呼ぶ」は誤りです。代入は右辺を先に評価するため、`drops()` の間に保持されるのは `links` だけです（rustc の scratch で確認）。
  - 抜け: `with_inputs` は、validate/prepare の間ずっと全 slot ロックを保持して `Authority::descriptor()` と registered check を呼びます。
- 「supplied thread が待てるか」の列は、`PrepareContext` と `disconnect_signal` の入口から導きました（§1.2）。

**規則（KC-24a 拡張）**
「coordinator は、Module の slot ロック以外の自分のロックを保持している間、Authority・time handle・host clock・`DataLink` のメソッドを呼ばない。freeze gate は `cancel` と `DEVICE_LOST` 用の `now()` の間だけ保持してよい。」
- 規則を一文にし、「待てるロック」ではなく「slot 以外のすべて」で縛りました。待てるロックで縛ると、誰も待たない `links` を禁じられないためです。
- 例外は KC-24a の Ceiling に書きます。
- KC-44、KC-45、RS-6 step 6、KC-46a の文面案も §2 と §3.1 にあります。

**#120 の設計**
- `wake` は `schedule` をロックなしで呼び、返った handle を `wake_handles` リストに追加します。追加するロックはベクタ操作の間だけ保持します。
- 各エントリは callback の `fired` フラグを持ち、追加のたびに発火済みを刈ります。freeze は両方の実行で、リスト全体を取って cancel します。
- 却下した案が 2 つあります。
  - 置き換えた handle を `wake` が cancel する案は、gate 外の cancel になり RS-6/RS-7 に反します。
  - `scheduled` に統合する案は、1 イベントごとに 1 エントリ増えて無制限に膨らみます。
- `schedule` の返りから追加までの窓で freeze が取った handle は、次の freeze 実行（cleanup 側）が持ちます。最後の freeze の後は、no-op のまま残ります。#111 の late-handle 規則は保たれます。

**#121 の設計**
- `links` を保持している間に `Arc<dyn DataLink>` を複製し、解放してから `drops()` を呼び、結果を `link_drops` に 1 回で格納します。
- RS-8a で step 6 が放棄された場合、Manifest の `drops` は全リンクで null になります（全部か無しか）。
- 却下: リンクごとに時間制限付きで呼ぶ案。`HostLink::drops` は atomic の読み出しなので、必要な実装がありません。

**ピン留め**
- `Shared` の `Mutex` を `Locked<T>`（現行の `Serial<T>` を一般化）に置き換え、全ロックを記録します（slot、`store`、`freeze_hook` は除外）。
- `Shared` に生の `Mutex` を足すと落ちる、ソース走査テストも加えます。
- sweep は、記録されたどのロックの下でも supplied 呼び出しを失敗にします。免除は (`Cancel`, freeze gate) の 1 組だけです。
- 新しいテストは T16、T17、T18、T15'、§4.2、§4.3、任意の T19 です。変更するテストは T11、T12、T13、T14 の 4 本です。
- 既存テスト 3 本が旧ロックの前提に依存していたので、監査表に載せました。
  - `kc_24a_freeze_takes_the_wake_handle_...` は §4.2 として作り直します。
  - `kc_24a_freeze_cancels_the_wake_handle_exactly_once` と T13 は、窓での競合を避けるため、待つ条件を「handle が追加された」ことに変えます。
- mutation row は M1 から M9 までで、すべて assertion で落ちます（ハングやタイムアウトでは落としません）。row N111-23 は削除し、N111-6 は付け直します。

**オーナー判断が要る点（推奨を先に）**
- **Q1** `DEVICE_LOST` の `now()` を slot と gate の下に残します（名前付きの例外）。
  - 残す理由は、そこで待てる supplied thread がないことです。
  - 動かす案は、slot が記録されない限り固定できません。slot を記録するには Module 呼び出し印の仕組みが要り、2 箇所のために過大です。
  - 例外ゼロにするなら、その仕組みを足す 1 ステップが追加で必要です。
- **Q2** `FreezePoint::BeforeWakeTake` を削除します。
- **Q3** `drops` は全部か無しかにします。
- **Q4** `Locked<T>` を型で強制します。`lock(&x)` を `x.lock()` に直す機械的書き換えが約 100 箇所あり、実装ステップ 1 でこれだけを先に入れます。

**INFERRED（実行して確かめていないもの）**
- Sink、Executor、gate を待つ supplied thread は存在しない。入口を読んだだけで、Module が実際にそれを試すテストはありません。
- freeze 後に `coerce` が始まることはない。KC-24 step 1 が先に拒否することを読んだのみです。
- 実機の paced Authority の `schedule` がブロックしうること。UHD のソースは読んでいません。
- 放棄された `drops()` が後で返っても無害であること。
- sweep の見えない範囲。どの Run も通らない経路、registered check や Provider への supplied 呼び出し、Module 自身が行う呼び出し。

Phase B のコードと、計画書の Sol レビューはまだです。

## Plan v2 and B1

計画 v2 の作成と B1 の実装を終え、ここで止めました。B1 のテスト・clippy・fmt・`rules.py` はすべて通り、行は phase7/G09 の 1 行を除き killed でした。コミットはしていません。

## 計画 v2
`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan-v2.md`

- **F1:** `assemble_manifest` は、outcome に `FlushEvents` の失敗がないときだけ `link_drops` を受け入れます。放棄後に遅れて返った `drops()` は Manifest に入りません。テストは timeout 記録後・assembly 前に `on_now` hook で wedge を解き、遅い書き込みの完了を `Reading.link_drops_len` で確認します。
- **F2:** T17 は `disconnect_signal()` の return で同期し、`finish` 後に handle ごとの cancel 数を見ます。実在しない M11 は M10 に置き換えました。
- **F3:** `kc_24a_freeze_cancels_the_wake_handle_exactly_once` で、unfired な通常 wake を early freeze の前に publish し、2 回の freeze をまたいで cancel 数 1 を確認します。
- **F4:** T18 は release ごとに control loop の新しい park を待ちます。最後の handle の publish は `wake_last_generation == wake_next` で確認し、その後に list 長を見ます。
- **F5(設計):**
  - **所有の境界:** coordinator が wake の handle を持つのは `schedule` が返した時点からです。list は `WakeList{handles, closed}` で、cleanup の freeze だけが `closed` を立てます。
  - **cancel の担当:** `closed` を見た publisher は、lock を持たずに自分で cancel します。data thread の end-wake は、`closed` を立てない early freeze の後に publish されるので生き残ります。
  - **追加の事前チェック:** `wake` の先頭で `closed` なら `schedule` しません。cleanup の step 2 中に data thread の wake が毎回 schedule と cancel を繰り返すのを避けるためです。
  - **却下:** 「gate の下で cancel / gate に渡す」は却下しました。Module thread が gate を取るのは禁止で、`try_lock` は stop が gate を持つときに失敗し、最後の freeze の後に list を読む者がいません。
  - **代償:** 「coordinator の cancel は常に gate の下」という性質を、この 1 経路で崩します。
  - **新規テスト:** T20(`closed` 側)、T21(open 側)、T22(事前チェック)。
- **F6:** 監視用 Authority は全メソッドを記録します。メソッドごとに non-vacuity を確認します。Q1 の例外は T19 で単独に pin します。coordinator は `pacing` / `governs` を呼びません。`device_paced()` は plan の class を読むだけです。
- **F7:** `kc_24a_freeze_runs_cancels_outside_the_admission_lock` を監査表に追加しました。trace から `BeforeWakeTake` を外し、lock 状態の直接 assert は残します。
- **F8:** inventory に `with_inputs` の `instance()` / `descriptor()` を足しました。prepare は `coerce` ではなく `instance()` を呼ぶと訂正しました。`links` の `clear()` が供給物の `Drop` を guard の下で走らせる箇所を足し、drop は guard の外へ出します。KC-44 は Module slot の `try_lock` 例外を明記します。

## B1
`git diff --stat`:10 ファイル、+196 / −145。

- **`Locked<T>`:** `Serial<T>` を一般化しました。`Shared` の `Mutex` は `store` と `freeze_hook` 以外すべて `Locked<T>` にしました。
- **機械的な書き換え:** `lock(&x)` を `x.lock()` に 71 箇所書き換えました。
- **lock record:** `Held.state` と `Which::State` を足しました。
- **テスト:** 単体テストに `held_records_a_locked_state_lock_and_only_that` を追加しました。ソース scan は `kc_24a_every_shared_lock_is_a_recording_lock` で、許可リストは `store` と `freeze_hook` です。

再アンカーした行は、`mutations.json` を `cefd067c` と id ごとに比較して拾った 11 行です。`rules.py` は `old` しか見ないため、`new` だけ変わった N103-9 が最初は漏れていました。

| 結果 | 行 |
|---|---|
| killed | F01, F02, G32, N103-1, N103-5, N103-8, N103-9, N111-3, N111-6, N111-25 |
| SURVIVED(known-survivors に記録済み) | phase7/G09 |

- **checks:**
  - `cargo test --workspace` は 3198 passed、0 failed です。
  - clippy は `--workspace --all-targets` と、testing feature なしの kernel lib の両方で `-D warnings` が通りました。
  - `cargo fmt --all --check` と `rules.py --base cefd067c` も通っています。

## 逸脱
- B1 は DeepSeek を使わず、sed と perl で自分で行いました。
- `Locked::new(value)` に名前文字列は持たせていません。そのため計画の `held_names()` は取りやめ、sweep の失敗メッセージは `Held` の Debug 表示にします。
- M9(`Locked::lock` が記録しない mutant)の行は B1 では追加していません。B2 で T16 とあわせて追加します。

## INFERRED
- wake の callback は Provider を呼ばないので、publisher の cancel が cleanup の stop と重なっても安全です。
- Module thread が Provider の slot を取ることはありません(`admission.rs:146`)。
- T17 と §4.2 は Attached Lease の Session で動かす必要があります。
- §4.3 の `CleanedUp` 遷移の `Now` が、`drops:entered` の後の最初の `Now` になります。
- 供給物の `DataLink` の `Drop` は sweep では見えません。

## 次の一手
B1 をコミットするか。コミットする場合は、`work/n120` 上に一度だけ。

## B2 and B3

B2 と B3 の実装、テスト、仕様文をすべて worktree `~/local_work/Ez-SDRv4-n111`(branch `work/n120`)に書き終えました。追加した行 N120-1〜15 はすべて assertion で kill されています。コミットはしていません。DeepSeek への委任もありません。

**diff --stat**: 13 ファイル、+1046/-200。
B2 と B3 は同じファイルに混ざっていて分けられないので、コミットは 1 つにまとめるのを勧めます。
- **B2**
  - `tests/support/start_window.rs`
  - `tests/support/run_doubles.rs`
  - `tests/coordinator.rs`(scan の allowlist 化、T16、drops の wedge テスト)
  - `ending.rs` の step 6、`assemble_manifest`、`links` clear の部分
  - `device_paced.rs` の T11〜T14 と T19
- **B3**
  - `state.rs`、`paced.rs`、`pipeline.rs`、`ending.rs`(`freeze_held`)、`mod.rs`(`FreezePoint`/`Reading`)
  - `device_paced.rs` の T17/T18/T20〜T22/F3/§4.2、`tests/support/paced.rs`
  - `design/04-run-and-session.md`、`design/06-kernel-coordinator.md`
- **両方にまたがる**: `mutations.json`(N120-1〜5 が B2、6〜15 が B3。G36 と N111-6 の付け替えと N111-23 の削除も含む)

**行の結果**(`remote_mutate.py` で sim02/sim03 実行、ログは `~/.cache/ezsdr-fixes/2026-11-20-n120/b2-mut.log`、`b3-mut.log`、`b3-mut2.log`、`b3-mut3.log`)
- **N120-1〜5(B2)**: 5/5 killed。
- **N120-6〜15(B3)**: 10/10 killed。
- N120-14 は最初 SURVIVED でした。変異側の wake は `schedule` の前に `wake_next` を数えますが、ログにはまだ `Schedule` が出ておらず、T22 の「`Schedule`/`Cancel` なし」が空振りしていたためです。T22 に次の 2 つを足しました。
  - `Reading.delivered_len` が増えたことの確認(空振り防止)。
  - `wake_next == 0` の assertion。
- 足したあとで N120-13 と N120-14 を再実行し、どちらも killed です。
- N120-12 と N120-15 は最初 ERROR でした(待ちがタイムアウトして infra marker が付いた)。assertion を待ちの前に移して再実行し、killed になりました。

**チェック**
- `cargo fmt --all` は完了。
- `cargo clippy --workspace --all-targets -- -D warnings` は OK(`needless_borrows_for_generic_args` を 1 件直しました)。
- `cargo test --workspace` は全 pass(ログは `b2b3-test2.log`)。
- 全コア busy で ×20(`device_paced` の kc_24a/kc_46a)は 20/20 pass でした。`coordinator` の kc_24a は 1 回だけ実行しました(14 passed)。
- `rules.py --base cefd067c --renames renames.txt`(テスト改名 2 件を記録したファイル)は 0 問題です。
- 敵対的セルフレビューは、新規テストごとに対応する行の kill で確認しました。

**逸脱**
- N120-4 の killer は T16 ではなく、lib の unit test `held_records_a_locked_state_lock_and_only_that` です。testing ビルドの guard 型が `Recorded` で、素の guard 変異が作れないため、`held()` の `state: state > 0` を `false` に変える行にしました。
- §4.3 の「drops がタイムアウト前に返る」ケースは、新しいテストを作らず既存の `ka_18_nonzero_link_drops_reach_the_manifest` に任せました。
- F3 の killer は既存の horizon fixture の変更ではなく新規テストです。その fixture は `next_wakeup` を回してコールバックを発火させるためです。
- T22 はクリーンアップのスレッドを `BeforeStop` ではなく `Done` で止めます。T21 の BeforeStop との順序は、probe 行(`freeze:BeforeStop:` と `cancel:<handle>`)で確認しています。
- M16(`links` clear の destructor 順)には行がありません。
- Terra の B1 指摘(source scan を `Shared` の全フィールド型の allowlist にする)は B2 で対応済みです。

**INFERRED**: プラン §7 のとおりで、追加はありません。

**オーケストレータ側に残るもの**
- `failure-mechanisms.md` と `n111-note.md` への追記。
- `rules.py` に `renames.txt` を渡すこと。
- 全 mutation list と gate。

B2 の短い記録は `~/.cache/ezsdr-fixes/2026-11-20-n120/b2-report.md` に置きました。

## Fix round 1

修正は完了し、未コミットで作業木に置いてあります（コミットはしていません）。

**`git diff --stat`**: 7 ファイル、+69 / -57。
- `paced.rs` と `state.rs`
- `tests/coordinator.rs` と `tests/device_paced.rs`
- `design/04` と `design/06`
- `plan/phase7/tools/mutations.json`

**指摘ごとの修正**
- **Terra 1 / Opus 3 / Opus 6（本体）**: `paced.rs` の `wake` は、リストが閉じているのを見つけた公開側が cancel を一切呼ばず、handle を捨てるだけにしました。wake は no-op のまま残ります。
  - 試験は 3 本を cancel 空の検査に直しました。
    - `kc_24a_a_wakes_schedule_holds_no_lock_and_blocks_no_one` の (6) は late の cancel が空であることを検査します。
    - 旧 `..._across_..._is_cancelled_by_its_publisher` は `..._is_never_cancelled` に改名し、cancel 空に変えました。
    - 旧 `..._accepted_before_..._is_cancelled_by_its_publisher` も `..._is_never_cancelled` に改名し、cancel 空に変えました。
  - この 3 本はいずれも、データスレッドが cleanup step 3 の join を終えた後に検査します。公開側の cancel があればログに載っている時点です。
  - 公開側の cancel を戻した変異は 2 行（N120-11, N120-16）とも assertion で殺しました。
    - N120-11 は T20 をねらった行で、変異が cancel を戻す箇所を再アンカーしました（結果は killed）。
    - N120-16 は「across」試験をねらう新行です（結果は killed）。
  - `remote_mutate.py` に N120-11 N120-16 を `simq` 経由で流し、2/2 killed でした。
  - 「no-op のまま残る」ことそのものの観測は、wake の callback が発火したかを見る手段がなく、cancel 無し・cleanup failure 無しで代えています。
- **Opus 2（RS-6 step 1 と KC-46a の線引き）**: 同じ言葉にしました。「wake の handle は wake list に入った時点で held。公開側がリストの閉鎖を見つけた wake は何も持たず、cancel せず、no-op のまま」とし、KC-46a は callback 発火後も no-op という一文を足しました。
- **Opus 1（KC-46a）**: 「その実行の後に始まる wake は何も schedule しない」に直しました。
- **Opus 4**: `PendingWake.generation` に `#[cfg(feature = "testing")]` を付け、生成側のフィールド初期化にも付けました。`testing` なしの `cargo check -p ezsdr-kernel` で警告は出ません。
- **Opus 5**: KC-24a の最初の文に「と下記の例外」を足しました。KC-44 の理由は「admission lock も freeze gate も取らず、他の自前ロックは supplied call をまたいで持たず、Module slot は `try_lock` のみ」に書き直しました。
- **Opus 7**: RS-8a の既定 5 s の枠を、T20（accepted before … never cancelled）、`kc_46a_a_wake_after_the_cleanups_freeze_schedules_nothing`（T22）、across 試験の Deschedule コメントに書きました。
- **Opus 9**: T20 と end-wake 試験の重複していた `on_schedule_return` を `park_first_data_wake()` にまとめました。source scan は `Mutex<` / `RwLock<` / `Condvar` を含む型を、`Locked<` で包まれていない限り未知として拒否します。`OnceLock<Mutex<_>>` を拒否する自己検査も足しました。
- **Opus 8 / Terra 2**: コードは変えていません。N120-12/14/15 は full.log（02:23 開始）で確認済みで、gate で再確認する前提です。

**検査**
- `cargo fmt --all --check` は OK です。
- `cargo clippy --workspace --all-targets -- -D warnings` は無警告でした。
- `cargo test --workspace` は全部 ok で、失敗はありません。
- 変えた wake 試験 4 本を全コア busy の下で 20 回流し、20 回とも pass でした。
- `rules.py --base cefd067c --renames renames.txt` は 0 problem です。`renames.txt` の 1 行目の行き先を `..._is_never_cancelled` に更新しました。
- `testing` なしの `cargo check -p ezsdr-kernel` は警告なしです。

**逸脱**
- `mutations.json` は main 側の `plan/maintenance/tools/` ではなく、N120 行の置き場である `plan/phase7/tools/` を編集しました。
- 公開側が閉鎖を見つける分岐は `drop(list)` だけの形に残し、変異行の anchor をその形に合わせました。
- 全 mutation リスト（fbfae8b3 分の full run）は、私の 2 行の前に流れ終わっていました。この修正後の全件実行は gate の仕事として未実施です。

**未確認**: 第三者 Authority の cancel と Provider の stop の直列化（Terra 1）は、公開側が cancel しなくなったので構造的に消えます。ただし「no-op の callback が最後の freeze の後で発火する」ことは、Authority の寿命が coordinator より長いという前提（INFERRED）に頼っています。

## Fix round 2

F1 から F5 まで直し、チェックと追加・再アンカーした行はすべて通った。コミットはしていない。

**`git diff --stat`**(12 ファイル、+287/-98)
- 本番コード: `coordinator/{mod,state,stepping,paced,ending}.rs`、新規 `coordinator/record.rs`(未追跡)
- テスト: `tests/coordinator.rs`、`tests/device_paced.rs`
- 設計文書: `design/04-run-and-session.md`、`design/06-kernel-coordinator.md`
- 変異行: `plan/phase4/tools/mutations.json`、`plan/phase7/tools/mutations.json`

**指摘ごとの対応**
- **F1(Simulation の DeviceLost でフォールトの Mutex を保持したまま `now()` を呼ぶ)**
  - 修正: `Fault` を `Locked` にし、ベクタを別文で取り出してから `apply_faults` を呼ぶ。
  - テスト: `kc_24a_sweep_a_simulation_device_lost_stamps_its_event_under_no_lock`。修正前は生の Mutex のため通ってしまうので、`Fault` を `Locked` にした状態で失敗を確認した。
  - 行: N120-17 は killed。D25 は `lock(&fault.0)` の書き換えに合わせて再アンカーし、killed。
  - スキャンテストに `struct Fault(Locked<` の確認を足した。
- **F2(供給 `cancel` が `disconnect_signal` を呼ぶと、ゲート保持中に `wake` が `now()`/`schedule()` を実行する)**
  - 修正: 保持中のロックの数をスレッドごとに全ビルドで数え、保持しているスレッドの `wake` は何もスケジュールしない。その数える部分を新規ファイル `record.rs` に分けた。
  - `Held` と `held()` は、`lock_record` の `testing` 専用インラインモジュールに残した。
  - テスト: `kc_24a_a_cancel_that_calls_disconnect_signal_schedules_nothing_under_the_gate`。修正を一時的に外すと、ゲート保持中の `Now` を検出して失敗する。修正後は通り、終了要求も処理される(`wait_for` は client_disconnect を返す)。
  - 行: N120-18 は killed。N120-4 は `mod.rs` のままで、killed。
  - KC-24a と KC-46a に、この規則を一節ずつ書いた。
- **F3(#121 のテストは、先に解放してから Manifest 組み立てを試している)**
  - 変更: 共通ヘルパー `run_with_wedged_drops(early_release)` を抜き出した。既存テストはそのまま通る。
  - テスト: `kc_24a_a_wedged_drops_parked_until_the_manifest_arrives_does_not_hold_it_up` を新設。`drops()` を停止させたまま Manifest の到着を待つ。
  - 行: N120-19 は N120-1 と同じ変異で、`"kill": "bound"` を付けた。outcome は「killed (bound)」。表明ではなく 30 秒の infra 上限で殺す。
- **F4(`kc_46a_the_wake_list_is_bounded` の 10 秒待ち 3 つに infra マーカーがない)**
  - 修正: 3 つとも `infra(...)` を付けた。N120-8/9 は表明(`wake_handles == 1`)で殺されており、どちらも killed。
- **F5(RS-6 の Changes 行と、`state.rs`・`ending.rs` のコメントが古い)**
  - 修正: 3 か所とも、締め切り後に公開された wake はキャンセルせずノーオペのまま残る、という書き方に合わせた。

**チェック**
- `cargo fmt --all`: 問題なし。
- `cargo clippy --workspace --all-targets -- -D warnings`: 通る。
- `cargo test --workspace`: 全部通る。
- `cargo check -p ezsdr-kernel`(`testing` なし): 警告なし。
- 全コアを埋めた状態で、新規 2 本と `wake_list_is_bounded`、F1/F3 の 2 本を 20 回ずつ流した。失敗はゼロ。
- `rules.py --base cefd067c --renames renames.txt`: 0 problems。
- 変異行は `remote_mutate.py` で流した(ログは `fix2-mut-p4.log` と `fix2-mut.log`、保存先は `~/.cache/ezsdr-fixes/2026-11-20-n120/`)。
  - phase4 の D25 は killed。
  - phase7 は N120-1、2、3、4、8、9、17、18 が killed、19 が killed (bound)。

**逸脱**
- 敵対的セルフレビュー(`sonnet-review-adversarial.md`)は走らせていない。変更内容を読み直しただけ。
- `wake` が何もしない条件は、スレッド局所のロック数が 0 でないこと。別スレッドが保持しているロックは見ない。意図どおりだが、Q1 の例外(スロットロック下の DEVICE_LOST `now()`)は `Locked` ではないので数えない。

**INFERRED**
- F2 でロックを保持している側は、解放後に終了要求を読み直す。出発点は撤回と early stop、それにクリーンアップ凍結の 3 経路で、検証したのは撤回経路(テストが `wait_for` の結果で確認)だけ。残る 2 経路は、終了要求が先に立っていることを読んで確かめた。

主なファイルは `~/local_work/Ez-SDRv4-n111/crates/ezsdr-kernel/src/coordinator/record.rs`、`paced.rs`、`stepping.rs`、`~/local_work/Ez-SDRv4-n111/crates/ezsdr-kernel/tests/coordinator.rs`、`device_paced.rs`。
