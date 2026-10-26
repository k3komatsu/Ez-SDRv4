# 段階 3c-2a：報告

## 実装の報告（`report-3c2a.md`）

STATUS: done

## Phase 8 stage 3c-2a

### Changes

- `crates/ezsdr-radio-uhd/tests/common/measurement.rs`: added raw timed-apply queue-depth trials for five conditions and raw coercion read-backs for gain, gain step, antenna, sample rate and frequency observations. Frequency samples remain observations only. Added conservative classification, queue defaults, and restoration that attempts every channel and returns all apply errors.
- `crates/ezsdr-radio-uhd/tests/common/mod.rs`, `tests/fake.rs`, `tests/hardware.rs`: shared `SAFE_TX_HZ`, integrated the groups into fake rehearsal and the hardware record, asserted observation fields, and tested restoration continuing after an apply failure.
- `design/18-uhd-radio.md`: updated UR-34's measurement behavior, test citations and Changes entry.
- `plan/maintenance/tools/mutations.json`: added P807–P839, retargeted P732, corrected P810/P824/P831 mutant replacements after their first compile errors, and normalized the touched JSON indentation.
- A clippy-only test cleanup uses `MeasurementPlan` struct initializers instead of post-`Default` field assignment.

### Brief clarifications

- The review verified that `FakeDevice` does have a timed-command queue with capacity 16; the rehearsal caps at 8. Its 10 ms host-time stop can nevertheless end a trial early under scheduling delay. The rehearsal therefore checks for a nonempty call list no longer than the configured maximum, not exactly 8 calls. It still checks trial counts and observation structure, and FakeDevice records remain unmeasured.
- Removed the restore-time check that compared the literal 1 GHz default against `SAFE_TX_HZ`; that constant is always in-band.
- No production `src/` code, new FakeFault, retry, or hardware test was added or run.

### Keys and tests

- `radio.timing.command_queue_depth`: `common::measurement::tests::command_queue_depth_requires_backpressure_timing_and_success_in_every_condition` covers missing/short/multiple stalls, slow earlier calls, call errors, the untimed reset error, minimum selection and insufficient conditions. `common::measurement::tests::measurement_plan_from_env_uses_defaults_and_rejects_invalid_counts` directly pins the default five trials.
- Gain, gain step, antenna and sample-rate coercion: `common::measurement::tests::coercion_readbacks_keep_only_exact_gain_rate_and_antenna_values`; profile-grid comparison: `common::measurement::tests::gain_grid_comparison_rejects_a_profile_point_missing_from_readback`.
- Frequency observations, unmeasured explanations and FakeDevice non-claims: `common::measurement::tests::measurement_record_has_exact_unmeasured_keys_and_valid_conditions` and `provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements`.
- Restore continuation: `measurement_defaults_restoration_attempts_all_channels_after_an_apply_error`.

### Hardware time and call estimate

At defaults, queue probing is 5 conditions × 5 trials. Each trial schedules commands 3 s ahead, spaced 1 ms, and drains through the last accepted command plus 10 ms: about 3.01–3.073 s of device time per trial, or roughly 75–77 s total before call overhead. It performs one setup apply, up to 64 timed applies and one untimed reset per trial (maximum 1,650 applies), plus `time_now` reads and drain polling.

For a one-RX/one-TX X3xx layout, coercion performs about 2,109 `apply` calls plus the profile's antenna count: 64 gain points and 316 fine gain points per direction, 639 sample-rate points per direction, 50 RX/20 TX frequency samples and one safe TX setup. UHD call latency—especially across the rate changes—was not measured, so total wall-clock bench time is unknown. No hardware record or measured device value was produced.

### Mutation and checks

- Final selected remote mutation run: **46/46 killed** (P719, P732, P735, P737–P741, P760–P761, P800, P804–P805, P807–P839). The three compile-error mutants P810/P824/P831 were corrected and killed on the final run. All 1,188 mutation `old` strings match the tree exactly once.
- Pure measurement tests: 27 passed. Restoration test: 1 passed. Final-tree fake rehearsal: 20/20 passed.
- `cargo test --workspace --no-fail-fast`: 1,804 passed, 0 failed.
- Workspace clippy and Docker UHD-feature clippy: passed.
- `rules.py --base HEAD`: 0 problems; `rules.py --self-test`: passed.
- `git diff --check`: passed.

### Self-review

One read-only self-review was completed with `opencode-go/deepseek-v4.1-flash#max`; the free model was unavailable due to its usage limit, so the same review was run once with Go. All findings were addressed:

- P1: removed the fake rehearsal's host-time-sensitive exact call-count assertion and reran 20 times on the final tree (20/20).
- P2: added mutation coverage for no stall and slow earlier calls (P835–P836), and a pure test plus mutation for a failed final untimed reset (P837).
- P3: aligned UR-34 behavior and test citations; normalized mutation JSON indentation; pinned gain/antenna and sample-rate not-run reasons (P838–P839); removed the dead constant safety check.

### Remaining

Hardware bench measurements and the separate Sol review remain outside this implementation run. Changes are unstaged and uncommitted; nothing was pushed.

## 修正の報告（`fix-3c2a-report.md`）

STATUS: done

## Phase 8 stage 3c-2a — review fixes

### Findings addressed

- **Sol 1 (major), queue classification:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:830-841` now rejects a trial if any earlier timed `Device::apply` lacks a device-time read-back or returns at/after the first scheduled command. The stalled call must return from `first_at` through `first_at + 100 ms`, inclusive; overflow of the bound is incomplete. Reported device-call/time-read errors remain device failures. Existing earlier-call wall-duration, single-stall, apply-error and untimed-reset checks remain. The synthetic test `command_queue_depth_requires_backpressure_timing_and_success_in_every_condition` (`measurement.rs:2887-2925`) covers earlier returns at/after the first command, a late stalled return, missing times, both inclusive boundaries, and existing failure/minimum rules.
- **Depth interpretation and evidence:** the observation (`measurement.rs:1049-1058`) says depth counts high-level timed `Device::apply` calls, not device FIFO entries, and marks attribution `INFERRED`: host call duration plus the returned device time do not directly prove a credit wait. Sol verified that UHD 4.10 OBX timed tunes can emit multiple register/synchronization writes (`obx_expert.cpp`, `obx_cpld_ctrl.cpp`); UR-24 counts one slot per channel-keyed Provider command (`design/18-uhd-radio.md:256,261`), so the operation-level quantity is the comparable one.
- **Sol 2 (minor), UR-34 history:** removed “from stage 3c-2a” from the normative UR-34 text (`design/18-uhd-radio.md:351`). The stage history remains in the Changes row (`:695`).
- **O1 (minor), sample-rate stream keys:** `channels::Description` contains no stream-membership mapping (`crates/ezsdr-radio/src/channels.rs:97-108`; membership is supplied separately in configuration at `:548-557`). Per the brief’s fallback, the existing channel-indexed key is retained and documented with a `shortcut:` comment (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1337`). Revisit when the layout description supplies stream membership.

### Bench estimate

At defaults, queue probing has 5 conditions × 5 trials. Each trial covers about 3.01–3.073 seconds of device time (3 seconds ahead, up to 63 ms command spacing, and a 10 ms drain margin), or roughly 75–77 seconds before call overhead. The maximum is 1,650 `apply` calls including setup and reset. For one RX and one TX channel, coercion performs about 2,109 raw `apply` calls plus the profile’s antenna count; restoration adds two calls. UHD wall time, especially for sample-rate changes, was not measured.

### Verification

- Queue classifier unit test: 1 passed.
- Final-tree FakeDevice rehearsal `provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements`: 20/20 passed.
- `cargo test --workspace`: 1,804 passed, 0 failed across 68 test targets.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problems.
- `git diff --check`: passed.
- Final selected remote mutations: **27/27 killed** — P807–P813, P820–P825, P832–P837 and P840–P847. All 1,196 mutation `old` strings match the tree exactly once.
- The fix did not change `tests/hardware.rs`; Docker UHD-feature clippy was not rerun. The implementation report `report-3c2a.md` records its earlier pass.

### Remaining

- No USRP hardware measurement was run; queue back-pressure attribution remains inferred until bench evidence is collected.
- Orchestrator note O2 remains for the owner before bench: confirm the TX gain sweep’s RF-safety conditions and record the decision in `bench.md`.
- Sol re-review was left to the orchestrator, and no self-review was run, as directed.
- Changes remain unstaged and uncommitted; nothing was pushed.

## 修正 2 の報告（`fix2-3c2a-report.md`）

STATUS: done

## Phase 8 stage 3c-2a — second fix

### 修正

- **Sol 再レビュー major（queue の観測が測定対象を乱す）**：`crates/ezsdr-radio-uhd/tests/common/measurement.rs:817-868,934-1006`。各試行で最初の timed `apply` より前に `time_now` を1回だけ読み，前後の host 時刻と `d0` を記録する。timed `apply` の間では `time_now` を呼ばず，返りの host 時刻 `h_i` から device time の下限・上限を算出する。読みの往復が100 msを超えた試行は timed command を出さず incomplete とする。queue drain の待ちと後続の untimed reset は維持。
- observation：`measurement.rs:1030-1042,1090-1103` に `d0`，読みの往復，呼び出しごとの `h_i` と device-time bounds を記録し，host と device の同速仮定・drift 無視を INFERRED と明記。
- 合成テスト：`measurement.rs:2932-2975` を新しい `d0` / `h_before` / `h_after` / `h_i` 形式に変更。読みの往復が100 ms超，前の呼び出しの上限が `first_at` と同値，止まった呼び出しの下限が `first_at - 1 tick` の各 incomplete と，`first_at`・上限ちょうどの包含境界を固定。
- FakeDevice rehearsal：`crates/ezsdr-radio-uhd/tests/fake.rs:705-714` は新しい observation fields の形だけを確認し，時刻値による判定は追加していない。
- Mutation rows：`plan/maintenance/tools/mutations.json:11505-11512,11825-11862`。P808 を missing host timestamp 用に更新し，P840–P843 を今回の時刻境界用に付け替え。P832 と P844–P847 も含め，対象10行を remote mutation で実行し **10/10 killed**。

### 確認

- Queue classifier の focused test：1 passed。最初の `--exact` 実行は0件選択だったため，filter を外して再実行し1件を実行。
- FakeDevice rehearsal：20/20 passed。
- `cargo test --workspace`：68 test targets，1,804 passed，0 failed。
- `cargo clippy --workspace --all-targets -- -D warnings`：passed。
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`：0 problems。全 mutation `old` の一致検査も通過。
- `git diff --check`：passed。
- この修正では `tests/hardware.rs` を変更していないため Docker UHD-feature clippy は未実行。実機測定も行っていない。

### 残り

- host clock からの back-pressure 判定は INFERRED のまま。実機で queue-depth measurement を行っていない。
- 指示どおり Sol 再レビューとセルフレビューは起動していない。
- 変更は未 stage・未 commit・未 push。
