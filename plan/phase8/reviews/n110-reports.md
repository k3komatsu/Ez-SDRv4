# #110 + #113 — implementer reports (Sonnet supervising Go DeepSeek)

## Plan (phase A)

計画書を書きました（フェーズ A なのでリポジトリは無変更で、`git status` はクリーンです）。

**成果物**: `~/.cache/ezsdr-fixes/2026-11-20-n110/test-plan.md`
DeepSeek には読み取りの全数調査を 1 回だけ任せました（`ds-1-out.md`）。主要箇所はこちらでコードと突き合わせています。

**#110 の ActionId 変更**
- 現状、バッチの ID は `dispatch` で採番されます。終了要求がバッチの dispatch より前に来ると、バッチは `pipeline()` のローカル変数ごと破棄され、どこにも記録されません。
- 変更後は、`admit_schedule` が全エントリの admit を終えたあとに 1..n を採番します。1 件でも拒否されたら採番しません（all-or-nothing、Session と同じ）。
- 採番した ID は新設の `Shared.start_batch` に置きます。dispatch ループはロックを持ったまま、これを空にしてから各 Action を配ります。
- `freeze_held` は `start_batch` に残っている ID を、キューの ID と一緒に `undelivered` へ移して整列します。
- 却下した代替案（詳細は計画書 §2.2）:
  - 3 か所の early return でそれぞれ記録する。
  - Drop ガード。
  - batch ごと `Shared` に移す。
  - エントリごとに採番する。
- ユーザーに見える番号の変化は 2 点だけです。
  1. arm から dispatch の間に Module が `Abort` を投げたとき、その ID は 1 ではなく n+1 になります（返るのは投稿した Module だけ）。バッチは常に 1..n です。
  2. 破棄されたバッチが `undelivered` に載ります。
- Session、Module 提出、agenda の ID の値は変わりません。fixture や golden に ActionId を持つものはありません。
- 既存テスト 1 件を修正します（下の「判断が必要な点」の最初の項目）。

**#113 の両 Provider 確認（コード読みのみ、実行はしていません）**
- 修正は `rx_settle` の `if registered.is_some() { origin } else { t }` を `origin` にするだけです。送信側は元から origin で終わらせています（`mr_25_a_transmit_clock_ends_at_its_origin_on_a_stop_at_arm`）。
- UHD には `segments` 行がありません。UHD の clock は最初の block を publish する前に登録され、`expected` はリンクの応答にかかわらず進みます。そのため origin で切られた登録済み clock は存在せず、ブロックなしで切られたセグメントは clock も行も持ちません。UHD のコード変更は不要です。
- 設計文書の変更は RM-25 と MR-25 の各 1 文です。

**計画書の内容**: 順序（S1–S8、M1–M4）ごとに、テスト、assert、観測方法、失敗させる誤動作を書きました。mutation 行は N110-1〜6 と N113-1 です。`design/` 04/05/06/07/09 とスキーマ説明の変更案、既存テストの監査表（still right / fixed）も入れています。

**判断が必要な点**
1. device の `start()` 失敗（Failed{arm}）の後も、admit 済みバッチが `undelivered` に載ります。決定の文言どおりですが、`kc_18_a_start_failure_is_failed_arm` に assertion を足し、`kc_24a_start_an_end_before_the_check_dispatches_nothing`（`coordinator.rs:5068`）の `is_empty()` を `[ActionId(1)]` に直します。推奨は受け入れです。
2. end 1（admit_schedule の直後）を決定的に作るため、`start_probe::Point::Admitted` を 1 つ追加します。`assert_extent` は未知の Point を無視するので、既存テストは変わりません。推奨は受け入れです。
3. `mech:` ラベルは、`failure-mechanisms.md` を読んでいないため付けていません。分類はオーケストレーター側でお願いします。

**INFERRED の項目**
- UHD が登録済みの受信 clock を origin で終わらせることはない（`rx.rs:332-374`、`656-725` を読んだだけ）。
- `ProbeExecutor::submitting_from_start` が `Abort` をそのまま通す（S2b の前提。実装者が最初に確認）。
- freeze の実行元は `freeze_held` の 2 か所だけ（`ending.rs:336`、`paced.rs:143`）。`stepping.rs:433` は撤回であって freeze ではありません。
- DeepSeek の調査結果は §6 の表に出るテストについてだけ照合しました。合成 ID の一覧（`ds-1-out.md` §1-C）は 1 行ずつ読み直していません。

この計画では、テスト実行、敵対的レビューのスクリプト実行、Sol 向けの調整はしていません。

## Plan v2 and the code

#110 と #113 の実装が完了しました。コミットはしていません。

**v2 の場所**
- 計画書: `~/.cache/ezsdr-fixes/2026-11-20-n110/test-plan-v2.md`。Sol の指摘 F1〜F8 をすべて反映しています。
- DeepSeek の報告: `ds-2-out.md`。作業ツリー: `~/local_work/Ez-SDRv4-n111`。

**`git diff --stat`**: 26 ファイル、+525/-37。
- 本体コード: kernel の coordinator 5 ファイル、`event.rs`、`manifest.rs`、mock-radio の `lib.rs`。
- テスト 4 ファイル、design 5 ファイル、schema 9 ファイル、`mutations.json`。

**実装**
- **#110:** `Shared.start_batch` と `take_action_ids(n)` を追加しました。ids は admit_schedule の末尾で一括して取るため連続し、拒否された batch は id を消費しません。dispatch ループは `start_batch` を取り出して `dispatch_with(.., id)` で配ります。`freeze_held` は `start_batch` の残りを `undelivered` に足します。`Point::Admitted` を追加しました。
- **#113:** MockRadio の `rx_settle` の `None` 腕で、行の `ended_at` を常に origin にしました。UHD 側は変更なしです。FakeDevice の `rx_stop_untimed` と TX の clock-end テストを根拠にしています。
- design: spec 04/05/06/07/09 を更新しました。`ActionId` と `undelivered` の説明文を直し、schema を再生成しました。バージョンと CHANGELOG は触っていません。

**テストと mutation row**

事前に今のコードで走らせた結果は次のとおりです。
- 今のコードで失敗(想定どおり): S1、S2、S2b、S3、S4 の 1 回目、M1〜M3。
- 今のコードで通過: S5、S7、S8、S9、S4 の 2 回目。

実装後はすべて通過し、row は 15/15 が killed です。J181、J187、J193、J195、J198、N108-99 は元の anchor のまま一致しました。

| row | 壊した内容 | killer |
|---|---|---|
| N110-1 | freeze が batch を `undelivered` に移さない | S1 |
| N110-2 | dispatch が `start_batch` を空にしない | S5 |
| N110-3 | ids を entry ごとに取る | S7 |
| N110-4 | 配るとき ids を引き直す | S4 |
| N110-5 | ids を逆順にする | S4 |
| N110-6 | end を見たらループを抜ける | S9 |
| N110-7 | id を 1 個多く取る(空 batch に幻の id) | S8 相当の空 batch テスト |
| N110-8 | id が 1 つ遅れる | S1 |
| N113-1 | 行を cut の瞬間 `t` で閉じる | M1〜M3 |

row 番号は計画書 v2 と異なります。実装時に N110-6(ids なし)を捨てて番号を詰めました。

**チェック**
- `cargo fmt --all`: 差分なし。
- `cargo clippy --workspace --all-targets -- -D warnings`: 通過。
- `cargo test --workspace --no-fail-fast`: 失敗は `t200_bare_uhd_script_judges_a_run_and_bounds_it` の 1 件だけです。シェルを `setsid nohup` で起動したため SIGINT/SIGHUP が無視されたのが原因で、前景で再実行すると通過しました。
- `rules.py --base d4f840ff`: 0 件。
- S9、S1、S7 を 16 コア全部ビジーの状態で各 20 回: 失敗 0。
- Python スイートは未実行です。クライアントに手を入れておらず、schema は説明文だけの変更です。

**自己レビュー(新規・変更テストごと)**
- **S1:** freeze が batch を落とす変更で `undelivered==[1]` が最初に落ちます。Admitted に到達し BeforePublish に到達しないことは trace で確認しています。cancel は 1 回で `held.freeze` が true、admission と machine が false です。スレッドは pipeline 上だけです。
- **S2 / S2b / S3:** 同じ `undelivered` の assertion で落ちます。Abort の id も確認しています(S2b は `x:start:ok:2`、S3 は id 2)。
- **S4:** 2 回目の run が ids と entry の対応を確認します。N110-4 と N110-5 が id の行で落ちました。
- **S5:** N110-2 が `undelivered==[1]` で落ちます。
- **S7:** entry 1 は `test.grid=19.5` の coerce 拒否で、`SB-46` を理由として確認しています。cleanup 中に出す Abort の id が 1 であることも確認しています。
- **S8:** 空 batch で end を要求した場合に何も記録されないことを、新規テスト `kc_24a_start_an_end_after_admission_of_an_empty_batch_records_nothing` で確認します。N110-7 が落ちます。DeepSeek が追加した `kc_14_…` と `ma_05a_…` の assertion だけでは幻の id を捕まえられません。
- **S9:** hook が freeze の `Serialised` 到達を待つので、データスレッドの遅延は待ち時間が延びるだけです(10 秒で infra 扱い)。N110-6 は `undelivered` の assertion で落ちました。
- **M1〜M3:** N113-1 が `ended_at` の assertion で落ちます。

**ユーザーに見える変更(各 1 行)**
- batch の ActionId を admission 時に確定します。dispatch 前に end が来ると `termination.undelivered` に載ります。
- arm 後で dispatch 前に Module が `Abort` を出した場合、その id は 1 ではなく n+1 になります。ただし Abort を admission より前に出すと 1 のままです。
- `start()` が失敗した Run(`Failed{arm}`)でも、admit 済みの batch は `undelivered` に載ります(Q1 承認済み)。
- MockRadio で origin 以前に cut された受信 segment の行は、`ended_at` が常に origin になります。
- Manifest のフィールド構造は変わらず、`ActionId` と `undelivered` の説明文だけが変わります。

**逸脱**
- F2 の S9 は、Sol が書いた「A は受信済み」の形ではなく、deaf Provider で A と B の両方が `undelivered` に載る形にしました。受信側の assertion は「Provider が何も受け取らない」です。受信済みと `undelivered` の分割を直接確認するテストではありません。
- 本体コードの変更は私が書き、DeepSeek にはテストを書かせました。

**INFERRED**
- UHD の受信クロックが origin で閉じられることはない、という読み取りは実行して確認していません。
- S1 の far-future の stop は ratio 10:1 の宣言が必要です。1:1 だと 1 時間未満になり `far_future_stop` がハンドルを見つけられません。

**未実施**
- フルの mutation list、gate、Python スイートはオーケストレーターの gate に残しています。

## Fix round 1

#110 と #113 の修正ラウンドは、採用した指摘をすべて反映して完了しました。作業ツリーは `~/local_work/Ez-SDRv4-n111`(`work/n110`)で、未コミットです。

`git diff --stat` は 14 ファイル、+35/−14 です。内訳は次のとおりです。
- 本体: `pipeline.rs`、`event.rs`
- テスト: `coordinator.rs`
- 設計文書: `design/06`、`design/07`
- 変異行: `mutations.json`
- スキーマ: 8 ファイル

**指摘ごとの修正**
- **Terra 1 / Opus 7(行の追加):**
  - N110-9「`take_action_ids` が 1 つ少ない」を追加しました。`kc_24a_start_an_end_after_admission_records_every_entry_in_admission_order` が assertion で倒します。
  - N110-10「`admit_schedule` が id を保存しない」を追加しました。`kc_24a_start_an_end_after_admission_records_the_batch_as_undelivered` が assertion で倒します。
  - plan v2 §4 との番号対応は次のとおりです。
    - 実装の N110-6 は plan の N110-7(ループ break)。
    - 実装の N110-7(`..=`)と N110-8(1 つ後ろ)は plan にない追加行。
    - plan の N110-6(id を保存しない)は新 N110-10 に、同 N110-8(1 つ少ない)は新 N110-9 に対応。
- **Terra 2 / Opus 4:**
  - `ActionId` の説明を「Run のカウンタ(KC-24)が発行する。admit された Action、または Module の `Abort`(RS-15, RS-16)」に直しました。
  - 8 つのスキーマを再生成しました(kernel 3、radio 2、sink 2、server 1)。
  - kernel の `schema_freeze` では radio/sink/server 側が更新されないため、各クレートの test(`EZSDR_UPDATE_SCHEMAS=1`)でも再生成しました。
- **Opus 1:** `design/07` の UHD 側の引用を `ur_26_a_cut_at_a_queued_origin_stops_the_stream_at_once` に直しました。あわせて、TX の clock-end テストが origin で終わることも書き添えました。
- **Opus 2:** KC-18 の境界を「`Running` の公開に続く検査より前に要求された end」にしました。
- **Opus 3:** KC-24a に「破棄した Action に加えて、dispatch 前の start batch の id も `undelivered` に記録する」と「dispatch が admission lock の下で batch の記録を空にする」を追記しました。
- **Opus 5, 6:**
  - dispatch ループのコメントが admission lock を指すように直しました。
  - `zip` の前に `debug_assert_eq!(ids.len(), batch.len())` を入れました。
- **Opus 8:** `kc_14` の assert メッセージを「arm で失敗した Run は batch を admit していない」に直しました。gain 用ヘルパと `format!` の統合は「where cheap」にあたらないため見送りです。
- **Opus 9:** `dispatch` は独自の `fetch_add` のままです。Opus の note どおり、変更していません。

**チェック**
- `cargo fmt --all --check`: OK。
- `cargo clippy --workspace --all-targets -- -D warnings`: OK。
- `cargo test --workspace`: 失敗 0。
- `schema_freeze`: 緑。
- `rules.py --base d4f840ff`: 0 problems。
- 行は `remote_mutate.py maintenance sim02:7,sim03:7 N110-9 N110-10` で実行し、2/2 とも `killed` です(infra マーカーなし、bound ではない)。

**未確認:** 既存の N110-1 〜 N110-8 と N113-1 は再実行していません。コメントと `debug_assert!` の変更で行のアンカーが変わっていないことは、`rules.py` の通過で確認しただけです。

**参考:** 全行の変異実行(`full.log`)は a53a8917 に対するもので、まだ走り続けていました。N110-9 と N110-10 は、その実行が sim の順番待ち(simq)を抜けた後に走りました。

## Fix round 2

#110/#113 の fix round 2 は終わりました。Sol の F1〜F3 を直し、確認はすべて通りました。コミットはしていません（worktree `~/local_work/Ez-SDRv4-n111`、HEAD cecf798b の上の未コミット差分）。

`git diff --stat`: 4 ファイル、+19/-5。
- `crates/ezsdr-kernel/tests/coordinator.rs`
- `crates/ezsdr-kernel/tests/device_paced.rs`
- `design/07-radio-model.md`
- `plan/maintenance/tools/mutations.json`

**F1（major）** `kc_24a_start_an_end_between_two_pushes_records_both_ids`（S9）に、2 本目の push を確かめる assert を足しました。`effective_gain(&run) == 2.0` を、`finish_checked` の前に見ます。
- 新しい row は `N110-11` です。dispatch ループが、end 要求後に 2 本目の id を `start_batch` へ戻して `continue` します。2 本目は push されないまま、freeze が id を拾います。
- `N110-11` の結果は killed（`remote_mutate.py maintenance sim02:7,sim03:7 N110-11`）です。これが落ちるのは、追加した assert だけです。
- `N110-6` は触っていません。

**F2** 早期 end の watched cancel テスト（`kc_24a_start_an_end_after_admission_records_the_batch_as_undelivered` 内）の lock 記録を、`!admission && !machine && !state` に完全にしました。`state` は `start_batch` を含みます。これを破る row は足していません（Sol の指摘が test 強化だけだったため）。

**F3** `design/07` の RM-25 の引用文を直しました。cut の規則は両 Provider 共通と明記し、`ended_at` の行が origin に付く保証は MockRadio のものだと書きました。UHD は、origin の前に device を失うと stop 行が書かれず、長さ 0 の区間も残らない、と書いています。

**確認**
- `cargo fmt --all --check` と `cargo clippy --workspace --all-targets -- -D warnings` は通りました。
- `cargo test -p ezsdr-kernel` は全部通りました。
- 変えたテスト 2 本（S9 と早期 end の test）を全コア busy の下で 20 回ずつ回し、失敗は 0 です。
- `rules.py --base d4f840ff` は問題 0 です。

`cargo test --workspace` の全体と Python スイートは回していません（kernel の test と設計文書だけの変更で、client には触れていません）。
