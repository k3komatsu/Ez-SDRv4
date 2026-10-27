STATUS: done

段階 3f を実装しました。queue の深さは、条件ごとに成功が 5 回そろうまで、最大 15 回まで試行を続けます。

**作ったもの**（変えたのは `crates/ezsdr-radio-uhd/tests/common/measurement.rs`。`src/` は変えていません）
- `MeasurementPlan` に `queue_max_trials` を足しました。既定は 15、env は `EZSDR_MEASURE_QUEUE_MAX_TRIALS` です。
- `queue_trials` は名前を変えず、「要る成功の数」と doc に書きました。env 名も既存の試験もそのままです。
- validate は、上限 < 要る数を拒み、record の u32 の確かめを上限の側（`queue_max_trials * 3`）に移しました。
- loop は `while !queue_condition_done(...)` で回ります。この関数が、成功の数が要る数に達するか、上限に達したら止めます。
  - device の失敗と `raw.halted()` で止めるのも、交互の遅れ（試行の番号の偶奇）も今どおりです。
- `summarize_queue_depth` は成功の数だけで判定し、値も成功だけから取ります。
- `unmeasured` の理由の文には、最大 N 回と、条件ごとの「M successes in n trials」を書きます。
- observation には、条件ごとの `trial_count` と `success_count`、全体の `required_successes` と `max_trials` を足しました（`ambiguous_trials` は残しています）。record の `trials` は実際に流した数です。
- `plan/phase8/tools/run-record.sh` の env 転送の一覧にも新しい名前を足しました（指示書にない変更です）。

**指示書から変えた所**
- 指示書は「fake で全部成功なら各条件 5 回で止まる」としていましたが、fake はそもそも blocking を起こさず、成功が 0 回です。そのため fake の rehearsal は、各条件で上限まで流します（上限に達して `unmeasured` になる経路そのものを real loop で通ります）。
- 5 回目の成功で止まる側は純粋な試験でしか見ていません。fake では観測できません。
- rehearsal の `queue_max_trials` は 1 にしました。3 にすると、1 試行を前提にした fake の既存 4 試験（`a_started_receiver_that_returns_no_samples…`、`a_receiver_whose_samples_stop_short…`、`a_receiver_whose_samples_pass…`、`a_panic_on_the_command_thread…`）が落ちたためです。

**試験**
- 純粋な試験 `queue_trials_go_on_until_the_required_successes_or_the_trial_limit` を足しました。
  - 成功・あいまい・不完全・失敗が混ざる列で、3 回目の成功で止まり、以後を流さないことを見ます。
  - 上限 5 で成功が 3 回に届かないときは `unmeasured` で、理由の文に条件ごとの数が入ることを見ます。
- 既存の試験は新しい数え方に合わせて直しました。
  - `measurement_plan_from_env_uses_defaults_and_rejects_invalid_counts` は、上限の既定、下限（< 要る数を拒む）、env の 0 と 4 の拒否を見ます。
  - `provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements` は、各条件が上限まで流れ、`success_count` が 0 であることを見ます。
  - `queue_depth_streams_and_drains_the_receiver_only_for_receive_frequency` は、受信の開始・停止の数を上限に合わせました。
  - `queue_trials_alternate_two_target_delays_and_record_the_blocked_call_s_number` は、要る数 1・上限 2 にして、2 回流れることを見ます。
  - `command_queue_depth_requires_backpressure_timing_and_success_in_every_condition` は、`queue_depth_value` の新しい引数に合わせただけです。

**spec と bench**
- `design/18-uhd-radio.md` UR-34 の queue の規則に、「成功が要る数そろうまで、上限まで」と上限未達の `unmeasured`、observation の数を足しました。Test 行に新しい試験を載せ、*Changes* に 1 行（段階 3f）足しました。
- `plan/phase8/bench.md` に、env の説明と、raw group の所要「2〜7 分」（INFERRED）を書きました。1 試行 3〜4 s は机上の見積もりで、実測ではありません。

**mutation**（`remote_mutate.py maintenance,phase7 sim02:4,sim03:4`、11 行 11 killed）
- 新しい行は P1038〜P1043 の 6 行です。
  - P1038 は成功の数 `>=` を `>` に、P1039 は上限 `>=` を `>` に、P1040 は loop の止め方を成功の数でなく試行の数にします。
  - P1041 は validate の下限を、P1042 は上限の既定値を、P1043 は env の既定値を変えます。
- 変えたコードに合わせて、P811 と P828 の `old`・`new` を書き換えました。
- 当て直したのは P826、P827、P1002 です。
- 全 list の `old` がちょうど 1 回一致することも確かめました。

**確かめたこと**
- `cargo test --workspace` は全部通りました。
- 時刻に依る fake の 4 試験を 20 回流して、失敗は 0 でした。
- `cargo clippy --workspace --all-targets -- -D warnings` と `rules.py --base HEAD`（0 problem）と `git diff --check` も通りました。触ったファイルは mode 644 です（`run-record.sh` は元から 755）。

**残ったこと**
- 実機で 5 回そろう見込みは、12 回の読みの比率（成功 6・あいまい 3・不完全 3）からは高くありますが、まだ見ていません。
- Sol のレビューは未実施です。

`~/.cache/ezsdr-fixes/2026-10-26-stage3/report-3f.md` は 2 行だけの短い版で、詳細はこのメッセージです。

---

STATUS: done

Sol の major 1 を直しました。`ValueRecord::Measured.trials` は、値を決めた条件が実際に流した試行の数になります。5 回の成功に 8 回かかった条件なら `trials: 8` です。

**変えたもの**
- `crates/ezsdr-radio-uhd/tests/common/measurement.rs` の `summarize_queue_depth` は、返す数を成功の数から `trials.len()` に変えました。値と条件の選びは成功だけのままです。
- `:3986` の assert は、`mixed`（成功 8・あいまい 2・成功 9）で `Some((8, "a", 2))` と誤った数を固めていたので、`3` に直しました。
- `queue_trials_go_on_until_the_required_successes_or_the_trial_limit` に、成功と失敗が混ざる 8 試行の列を足しました。ここで `Measured` の `trials: 8`、condition `"a"`、worst `8.0` を assert します（比べる相手は条件 `b`：成功 2 回で値 12）。
- `plan/maintenance/tools/mutations.json` に P1044 を足しました。返す数を `queue_successes(trials)` に戻す mutant です。

**確かめたこと**
- mutation は P811、P1044、P1038、P1039 の 4 行が killed です（`remote_mutate.py maintenance,phase7 sim02:4,sim03:4`）。P811、P1038、P1039 は触っていませんが、同じ関数の近くなので当て直しました。
- 全 list の `old` がちょうど 1 回一致することも確かめました。
- `cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`rules.py --base HEAD`（0 problem）、`git diff --check` が通りました。
- 触ったファイルは mode 644 です。
- 変えたのは純粋な試験だけなので、20 回の繰り返しは流していません。

`~/.cache/ezsdr-fixes/2026-10-26-stage3/report-3f-fix1.md` は 2 行だけの短い版で、詳細はこのメッセージです。