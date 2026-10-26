# 段階 3b-1：テスト計画と報告

## テスト計画（`test-plan-3b1.md`）

STATUS: done

## 段階 3b-1 — テスト計画

これはテスト計画のみ。3b-1 は候補 timing の配線、証拠の分類・読み方、raw observation、record の構築と安全な書き出し、FakeDevice のリハーサル、ignored hardware entry point に限る。Provider の測定 sweep（最小 command lead、in-flight、RX start lead、throughput、full duplex）は 3b-2 の別計画に送る。3b-1 の `Record.values` は、比べる key を漏れなく一度ずつ含め、**すべて `unmeasured` と理由を記録する**。raw probe の数値も比較値にしない。

実装順は、純粋な分類・集計テスト → builder の default/候補配線と controlled-clock runtime tests → record/output → FakeDevice rehearsal → ignored hardware entry point。hardware の時間値を FakeDevice から推定しない。

### 確認済みの根拠と境界

- `TimingEnvelope` は 8 field（`min_timed_command_lead_ns`, `startup_latency_ns`, `command_queue_depth`, `release_window_ns`, `overflow_restart_gap_ns`, `rx_start_lead_ns`, `tx_start_lead_ns`, `tx_in_flight_ns`）を持つ（`crates/ezsdr-radio/src/lib.rs:721-739`）。UHD construction は Description から capability tree、`min_command_lead`、`envelope` section を作る（`crates/ezsdr-radio-uhd/src/provider/mod.rs:263,308-321`）。Run 後には `applied`, `bursts`, `async`, `rejected`, `timing`, `stats` が記録される（同 `:400-416`）。
- Runtime reader は `release_window_ns` と `command_queue_depth`（`provider/control.rs`）、`tx_in_flight_ns`（`provider/tx.rs`）、TX/RX start lead（`provider/tx.rs`, `provider/rx.rs`）、`startup_latency_ns`（`provider/mod.rs`）にある。`DEVICE_LEAD_NS` と `DELIVERY_ALLOWANCE_NS` は別の Module 定数であり、候補 builder から変えない。
- `measurement::compare` は各 comparable key の一意性・網羅、`unmeasured` の reason、condition の基本規則を検査する（`crates/ezsdr-radio/src/measurement.rs:394-515`）。condition が実運転を正しく表したかは、record を作る側の assertion で別に確かめる。
- `applied` の instant は予約時刻で、実際の装置実行時刻の観測ではない。従って queue depth の比較値は `unmeasured` とし、`hw_b8_raw_queue_depth` は observation に残す。
- FakeDevice は manual clock。試験 thread が時計を読んだ後の遅延で verdict を決めない（`crates/ezsdr-radio-uhd/tests/README.md`）。

### テスト系列

| # | test（新規／拡張） | 場面・assert | 観測箇所 | 落とす誤実装 |
|---|---|---|---|---|
| 1 | **拡張** Provider unit test + `ur_09_profile_values_reach_the_capabilities_and_the_envelope_section` (`tests/fake.rs`) | Builder を呼ばない UHD construction と短い Run。全 8 timing field が従来の `x3xx::timing(profile::terms())` と一致し、既定の動作も変わらない。 | unit test で private Description、integration test で `ProviderInstance.min_command_lead`、capability tree、instance の `envelope` section、Run 後 Manifest の envelope。 | builder の候補が暗黙に既定へ漏れる、または既定値の一部だけ変わる。 |
| 2 | **新規** `measurement_candidate_propagates_all_timing_fields` (Provider unit + `tests/fake.rs`) | `assembly(..., adjust)` から builder を呼び、8 field すべてに互いに異なる有効な sentinel 値を渡す。Description、capability、envelope section、Manifest envelope が各 field の候補値と一致し、`min_command_lead` が候補 lead と一致する。 | Description の直接 assertion、`ProviderInstance` surfaces、Run 後 Manifest。8 field を field ごとに比較する。 | Description だけ書き換わる、capability/envelope が古い、field のコピー漏れ、`min_command_lead` だけ古い。 |
| 3 | **新規** `measurement_candidate_is_read_by_each_existing_runtime_reader` (controlled-clock Provider rigs) | reader ごとに独立した rig と観測を置く。①release window: candidate window の境界前後で command が device に渡る tick が候補どおり。②queue depth: candidate=N で N 件目まで受理し、N+1 件目が queue-full。③TX in-flight: timed TX/repeat の handoff 上限が候補どおり。④TX start lead: first TX sample の開始可能 tick が候補どおり。⑤RX start lead: first RX sample の開始可能 tick が候補どおり。⑥startup latency: arm/start から stream 開始までが候補値どおり。比較値の sweep や実機性能の主張はしない。 | 各 rig の controlled clock と FakeDevice の handoff/start/sample tick、該当する Provider event/section。reader ごとに期待値を個別 assertion する。 | 候補が capability には出るが runtime が既定値を読む、TX/RX の lead を取り違える、壁時計に依存する。 |
| 4 | **新規** `reject_at_plan_uses_candidate_admission_floor` (`tests/fake.rs`) | `Spec.schedule` の `RejectAtPlan` timed burst を使う。同じ 4 ms lead は既定 5 ms で拒否され、候補 3 ms では通る。Session `send` の `DropAndFlag` を admission floor の証拠に使わない。 | compile/admission の rejection または admitted action。候補値を含む Manifest も照合。 | Session send の acceptance を誤って admission lead の検査と見なす、candidate が admission に届かない。 |
| 5 | **新規** `candidate_admission_does_not_change_device_lead` (Provider controlled-clock rig) | 候補 admission lead を 1 ms にし、2 ms より近い timed burst（例 1.5 ms）を固定時刻で submit。admission は通るが `DEVICE_LEAD_NS` の Provider 判定で失敗し、device には送信されない。試験 thread の現在時刻で target を決めない。 | submit outcome、Provider `TIME_ERROR` の outcome、device の send 呼び出し数（0）。device report はこのケースに要求しない。 | 候補が `DEVICE_LEAD_NS` まで書き換える、Provider drop を admission rejection/成功扱いする、落とした burst を device に送る。 |
| 6 | **新規 pure tests** `classify_trial_failure_origin` と `trial_reports_must_be_attributable` (`tests/common/measurement.rs`) | 合成 evidence を一件ずつ与え、6 outcome を網羅する。admission rejection。Provider failure（`TIME_ERROR` の Provider-origin outcomes `drop`/`send_asap`/`refused`/`plan_violation`、`LATE_COMMAND`、`COMMAND_REJECTED`、`COMMAND_QUEUE_FULL`）。Device failure（`TIME_ERROR { outcome: late_at_device }`、対応する／しない async `TimeError`、`Underflow`、`UnderflowInPacket`、`SeqError*`）。Incomplete（error は無いが record/completion が無い、sample 数/end が違う、短い send の UR-22 `rejected` note）。Unattributable（区間外 report、tick 無し、複数 trial にまたがる ack）。Success（必要な record・全 sample・完了が揃い、error/evidence ambiguity が無い）。matched `late_at_device` event も device failure で、event の搬送元だけで Provider failure にしない。 | 分類結果と保存した raw evidence。試行は順番に一つずつ実行し間隔を置く。tick 付き report は試行区間内だけ帰属。tick 無し／区間外は unattributable として、開いている試行（無ければ直前）を不合格にする。ambiguous evidence は成功にしない。 | async を無視、`late_at_device` を Provider-origin と誤分類、欠落／短い burst を success、前後 trial の report/ack を誤帰属。 |
| 7 | **新規 pure tests** `minimum_lead_uses_successful_suffix` / `capacity_uses_successful_prefix` / `incomplete_trials_never_pass` | lead は候補以上の全段が全試行成功する最小段、capacity は最小段から連続して全試行成功する最大段。例 `[pass, fail, pass, pass]` の lead は 3 段目、capacity `[pass, fail, pass]` は 1 段目。全段失敗は reason 付き `unmeasured`。段の規定試行数に届かない場合は pass にしない。複数 condition にまたがる値は、各 condition で成功した候補の共通部分だけを採る。 | pure summarizer の `worst`/unmeasured reason、trials、condition ごとの全件 counts。cold-change before/after は合成入力で intersection を試すだけで、3b-1 で実測しない。 | 最初の偶然の成功、capacity の後段孤立 pass、試行不足、条件一つだけの成功を採用する。 |
| 8 | **新規** `raw_probe_results_are_observations_only` (`tests/common/measurement.rs`) | `hw_b8_raw_device_lead`, `hw_b8_raw_restart_lead`, `hw_b8_raw_queue_depth`, `hw_b8_raw_in_flight_window`, `hw_b8_raw_burst_gap`, `hw_b8_raw_empty_eob_gap`, `hw_b8_raw_timed_tune_phase`, `hw_b6_delay_by_rate` の総遅れ、`hw_b8_stop_end` の結果を observation に変換する。従来の print/assert と probe 回数は維持。 | serialized `observations`; comparable `values` に raw 値が一切無く、全 value row が `unmeasured`。 | raw 値を measured capability にする、loopback delay を TX/RX path delay に割り振る、既存 assertion を落とす。 |
| 9 | **新規** `measurement_record_has_exact_unmeasured_keys_and_valid_conditions` (record builder + compare tests) | profile/layout の全 `rule(key) != NotCompared` をちょうど一行ずつ生成。3b-1 はすべて `Unmeasured` で reason 非空。`NotCompared` は書かない。condition ID と `(direction, stream)` の一意性、stream/channel/rate layout、`concurrent` の参照、各 observation の condition 参照を検査。hardware-source の合成 record を Mock と UHD Description の双方で `compare` し、構造・condition 違反がないことを確認。 | `Record.values`, `conditions`, `observations`、JSON round-trip、両 Description に対する `compare` violations。 | key の欠落/重複/未知値、空 reason、無効な condition、raw 値の混入、比較器だけで実運転条件まで証明したと誤認。 |
| 10 | **新規** `measurement_metadata_and_output_are_fail_closed` (pure validation + writer tests) | `EZSDR_MEASUREMENT_OUTPUT`, `EZSDR_MEASUREMENT_MBOARD`, `EZSDR_MEASUREMENT_FPGA_IMAGE`, `EZSDR_MEASUREMENT_HOST_NAME`, `EZSDR_MEASUREMENT_SETTINGS_SCRIPT`, `EZSDR_MEASUREMENT_SETTINGS_VERSION`, `EZSDR_MEASUREMENT_LOAD`, `EZSDR_MEASUREMENT_COMMIT` の各必須 env を未設定・空で試し、writer 前に失敗する。`EZSDR_MEASUREMENT_LOAD` は `none`/`h4` 以外を拒否。serial/UHD version は device metadata、profile/selector は binding から得る。出力先なし/空でも fail（既定 path/skip/temp path は不可）。既存ファイルを指定したら上書きせず失敗し、元 bytes が不変。 | metadata validation result、writer error/success、既存ファイル bytes。env 読み取りは test lock または注入可能な env source で他 test と競合させない。 | 欠落値を placeholder で埋める、空値を通す、無言 skip、既存 record の上書き。 |
| 11 | **新規** `measurement_fake_rehearsal_round_trips_and_only_fails_source` (`tests/fake.rs`) | FakeDevice の manual clock で共通 record 経路を通し `source: fake` の record を temp file に serialize/read back。timed verdict は事前予約または controlled-clock rig で決め、test-thread clock read 後の遅延を使わない。`compare` は Mock/UHD の双方で `source` violation 一件だけ。既存 acceptance fake-source rejection（`rm_35_schema_source_and_profile_are_checked`）を維持・参照。 | round-trip record、全 comparable keys の unmeasured row、compare violations、acceptance fixture test。 | fake 結果を hardware measured として扱う、JSON 形だけで rehearsal 成功とする、source 以外の意図しない違反、acceptance が fake を受け入れる。 |
| 12 | **新規 ignored** `hw_measurements_record` (`tests/hardware.rs`) | USRP を明示的に使う ignored entry point。raw probe observations だけを集め、`source: hardware`、全 comparable value rows は unmeasured の record を、完全な `EZSDR_MEASUREMENT_OUTPUT` path に一度だけ書く。metadata 欠落/不正や既存 path は明示 fail。通常の CI では実行しない。 | 出力 JSON の source/values/observations/metadata、writer の non-overwrite behavior。manual hardware invocation で確認。 | 暗黙の出力先、既存記録の上書き、raw 値を measured にする、ignored entry point が無言 skip。 |

### 3b-1 の unmeasured reason

record builder はすべての comparable key に非空の reason を出す。少なくとも次を別扱いにしてテストする。

- `radio.timing.command_queue_depth`: `applied` は予約 instant で、装置の実行 instant を観測する道が無い。raw `hw_b8_raw_queue_depth` は observation のみ。測定方法は 3c で決める。
- `min_timed_command_lead_ns`, `tx_in_flight_ns`, `rx_start_lead_ns`, `rx_bytes_per_s`, `full_duplex`, `hardware_time`: comparative Provider measurement group は 3b-2 に延期。
- その他の timing/performance/repeat/channel keys（例 `startup_latency_ns`, `overflow_restart_gap_ns`, `tx_start_lead_ns`, `tx_bytes_per_s`, `wire_bytes_per_sample`, channel grids/path delay）: 3b-1 では測定していない。対応する独立測定または channel ごとの較正が無い限り unmeasured とする。`hw_b6_delay_by_rate` の合計遅延は path delay に分割しない。

### unresolved: `overflow_restart_gap_ns` の runtime reader

要求された builder/runtime coverage の grep で、UHD Provider 内に `overflow_restart_gap_ns` を読む runtime reader は見つからず、現時点で確認できたのは timing/envelope の宣言・伝播経路だけだった（`crates/ezsdr-radio/src/lib.rs`, `crates/ezsdr-radio/src/x3xx.rs`, `crates/ezsdr-radio-uhd/src/provider/`）。従ってこの計画では Description/capability/envelope/Manifest への候補値伝播はテストするが、runtime consumption test や overflow restart の振る舞いは**指定しない**。意味を新設したり、FakeDevice で挙動を捏造しない。owner が runtime semantics/reader の有無を決めた後、必要なら別のテスト変更として計画する。

### Mutation row の案

| 変異の狙い | 殺すテスト |
|---|---|
| builder の default 適用、または既定値/候補値の一つをコピーしない | #1, #2 |
| capability、envelope、Manifest または `min_command_lead` の一面だけ古い値にする | #2 |
| runtime reader のいずれかを Description 値でなく既定値に固定 | #3 の該当 reader assertion |
| `RejectAtPlan` admission floor を候補値でなく default に固定 | #4 |
| 候補 min lead から `DEVICE_LEAD_NS` を上書き、または drop を device send 扱い | #5 |
| `late_at_device` を Provider failure と分類、async error を無視、short/missing completion を success にする | #6 |
| trial interval/tick/ack attribution を緩める | #6 |
| lead suffix/capacity prefix/試行数/condition intersection の条件を外す | #7 |
| raw observation を measured value にも出す、raw assertion を削る | #8 |
| value key filter/uniqueness/reason/condition reference を壊す | #9 |
| env missing/empty/invalid を許す、または既存 output を truncate | #10 |
| fake source、serialization/readback、acceptance rejection を壊す | #11 |
| hardware entry point を skip/default path にする、raw observation を value に変える | #12 |

### 規模、INFERRED、未実施事項

- **規模の概算: 約 500–800 行**（test support、pure tests、builder/runtime checks、record/output、rehearsal を含む）。これは既存 `tests/common` と Provider rig を使える前提の **INFERRED** estimate で、LOC dry run はしていない。1,500 行以内の見込み。3b-2 の provider sweeps はこの見積もりに含めない。
- **INFERRED:** 実装時に既存 controlled-clock rig から各 reader の観測点を得られること。得られない reader test は壁時計で代替せず、必要な最小 test seam を見直す。FakeDevice の合格は hardware 性能の根拠にならない。
- **未実施:** この文書は計画のみ。コード、tests、mutation、hardware entry point は実行していない。確認時点の repo には `plan/phase8/00-overview.md` の変更があったため触れていない。

### 3b-1 implementation clarifications (2026-10-26)

- Trial completion is direction-specific: TX requires `BurstAck`; RX succeeds from the exact captured sample count and end tick. An out-of-range tick before the first trial has no preceding trial to fail.
- Series 5's controlled Provider test queues through `Tx::step` and observes the provider drop and zero `tx_send` calls. It does not repeat Kernel admission: the existing KC-19 test covers admission, while the candidate builder test asserts the candidate `min_command_lead` surface.
- The FakeDevice Run record is compared against the UHD description in `fake.rs`. Cross-provider source rejection is in `ezsdr-acceptance/tests/measurements.rs`, where both provider crates are allowed; `ezsdr-radio-uhd` must not gain a dev-dependency on MockRadio (MA-3/PO-8).
- A hardware-source record requires `uhd_version` from the device description and profile from the binding. The fake rehearsal supplies its non-applicable UHD version explicitly.
- The ignored hardware entry point is compile/clippy checked, not remotely mutated or run: its execution requires a USRP. Raw-observation, metadata validation, writer and all-unmeasured-value helpers are covered without hardware.

## 実装の報告（`report-3b1.md`）

STATUS: done

## Phase 8 stage 3b-1 report

### Summary

Implemented candidate-only timing propagation and the measurement-record/rehearsal support. Ordinary UHD construction remains profile-based. Every comparable record value is `unmeasured` with a non-empty reason; raw hardware probe results are observations only. No hardware test or measurement was run.

The work is on branch `main`, based at `e93a7fd81dca2fbe2a6024055694550609054c57`. Changes remain unstaged and uncommitted.

### Files changed

- `crates/ezsdr-radio-uhd/src/provider/mod.rs`, `control.rs`, `rx.rs`, `tx.rs`: candidate timing builder and propagation; controlled-clock checks for candidate readers, including directional starts, RX missed-start recovery, queue depth, release window, in-flight chunking and device-lead separation.
- `crates/ezsdr-radio-uhd/tests/common/mod.rs` and new `tests/common/measurement.rs`: measurement metadata/record construction, fail-closed writer, raw-observation collection, trial evidence attribution/classification and conservative candidate summaries.
- `crates/ezsdr-radio-uhd/tests/fake.rs`: candidate Manifest propagation and FakeDevice Run rehearsal with serialized/read-back record evidence.
- `crates/ezsdr-radio-uhd/tests/hardware.rs`: raw probe observations and ignored `hw_measurements_record` entry point.
- `crates/ezsdr-acceptance/tests/measurements.rs`: cross-provider check that a fake-source record is rejected by MockRadio and UHD only for its source.
- `design/18-uhd-radio.md`: measurement-only builder rule and named tests; Changes entry references design note 41, stage 3b-1.
- `plan/maintenance/tools/mutations.json`: measurement-path mutation rows.

All changed and added files are mode 644.

### Plan decisions and test coverage

| Series | Tests / disposition |
|---|---|
| 1 | `measurement_timing_candidate_updates_every_surface_and_the_start_boundary` pins ordinary construction to profile defaults; `ur_09_profile_values_reach_the_capabilities_and_the_envelope_section` covers the default Run. |
| 2 | `measurement_timing_candidate_updates_every_surface_and_the_start_boundary` and `measurement_candidate_propagates_all_timing_fields_to_the_run_manifest` assert all eight candidate fields across Description, capabilities, `min_command_lead`, envelope section and Manifest. |
| 3 | `ur_24_the_release_window_comes_from_the_description`; `ur_24_the_command_queue_is_counted_per_instance_across_streams`; `measurement_candidate_in_flight_window_sets_the_tx_chunk_size`; `measurement_candidate_directional_start_leads_set_cold_change_origins`; `ur_17_the_missed_start_lead_is_measured_from_the_decision_s_now`; and the start-boundary assertions in `measurement_timing_candidate_updates_every_surface_and_the_start_boundary`. These use controlled clocks and assert observed Provider/device state. |
| 4 | Removed as directed. Kernel admission is already pinned by `kc_19_a_reject_at_plan_burst_with_a_short_lead_is_refused`; series 2 verifies the candidate `min_command_lead` surface. No duplicate 3–5 ms integration test was added. |
| 5 | `measurement_candidate_admission_lead_does_not_override_the_device_lead` checks the Provider drop, `TIME_ERROR`, and zero device sends. Kernel admission is covered separately as above. |
| 6 | `classify_trial_failure_origin_and_completion_evidence` and `incomplete_or_unattributable_trials_never_pass` cover failure classes, direction-specific completion, ambiguous evidence and attribution boundaries. TX completion requires `BurstAck`; RX requires the exact captured sample count/end. |
| 7 | `candidate_summaries_use_successful_suffix_prefix_trial_counts_and_condition_intersection` checks successful suffix/prefix, trial count and condition intersection rules. |
| 8 | `raw_probe_results_are_observations_only` checks raw probe values stay out of comparable values while existing probe observations/assertions are retained. |
| 9 | `measurement_record_has_exact_unmeasured_keys_and_valid_conditions` checks exact comparable keys, reasons, conditions and record comparisons. |
| 10 | `metadata_and_writer_fail_closed` checks missing/empty/invalid metadata, H4 load acceptance and non-overwrite behavior. |
| 11 | `measurement_fake_rehearsal_round_trips_and_only_fails_source` ties the FakeDevice record to Run evidence. `fake_source_is_rejected_by_both_providers_only_for_source` checks both providers in the acceptance crate. Cross-provider checks live there to avoid a MockRadio dev-dependency in the UHD test crate. |
| 12 | `hw_measurements_record` is ignored and was compile/clippy checked only; it was not run because it requires a USRP and complete hardware metadata. |

The admission integration test from the original series 4 plan was intentionally omitted. `overflow_restart_gap_ns` is tested for propagation only because the UHD Provider has no runtime reader; its semantics remain for stage 3c. Provider measurement sweeps remain deferred to stage 3b-2. No comparative value was marked measured.

### Mutation results

All 38 relevant rows were killed: P682–P683 and P706–P741. The initial P710 run survived; the corrected test/row was rerun and killed. P735 initially produced a compile error in the 31/32 batch; the mutant was corrected and the single-row rerun killed it. `rules.py --base HEAD` also verified mutation `old`/`old2` matches exactly once.

Logs: `mutations-3b1.log`, `mutation-P710-final2.log`, `mutations-review-fix-final.log`, and `mutation-P735-final.log` in this work directory.

### Final checks

- `cargo test --workspace --no-fail-fast`: passed; 1,781 passed, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- UHD-feature clippy in `ezsdr-v4-dev:uhd4.10`: passed.
- `cargo test -p ezsdr-radio --test radio_model rm_20_schema_freeze`: passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problems; `--self-test`: passed.
- `git diff --check`: passed; all changed/added file modes are 644.
- Three controlled-clock tests were each repeated 20 times and passed: `measurement_timing_candidate_updates_every_surface_and_the_start_boundary`, `measurement_candidate_directional_start_leads_set_cold_change_origins`, and `measurement_candidate_admission_lead_does_not_override_the_device_lead`.

### Read-only self-review

Reviewer: DeepSeek V4.1 Flash Free (`deepseek-v4.1-flash-free`). Result: P1 = 0. All findings were either fixed or handled by the brief's adjudications:

- Fixed RX trial completion, fail-closed hardware metadata/profile sourcing, Provider drop/device-send observation, explicit unmeasured-reason/key assertions, and missing mutation coverage.
- The request to compare through a MockRadio dependency in the UHD test crate was not adopted. Fake Run evidence is checked against UHD in `fake.rs`; cross-provider source rejection is checked in acceptance tests, keeping provider dependencies separate.
- Fixed the default Description assertion, pre-first-trial out-of-range attribution, the spec sentence/Test citations, and the detailed findings (unused closure, hard-coded allowance, H4 acceptance, record identity assertions and hardware-entry mutation coverage).
- For the admission concern, the Provider test observes its own device-lead decision; Kernel admission remains independently covered by KC-19 and the candidate surface assertion, as specified in the brief.

### Not done

No hardware entry point, hardware test, or measurement was run; FakeDevice results are not hardware evidence. Python suites were not rerun (no Python source changed and they were not part of this stage's specified gate). Stage 3b-2 Provider sweeps and stage 3c overflow-restart semantics remain deferred.

## 修正の報告（`fix-3b1-report.md`）

STATUS: done

## Fix 3b-1 — implementation fixes

### Findings and changes

- **Sol 1 / O2 — candidate summaries:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:463-514` now derives each condition's successful suffix/prefix over all of its own stages, counts under-trialled stages as failures, and only then intersects eligible values. The minimum lead is the minimum of the intersection; capacity is its maximum. An empty intersection yields `unmeasured` with a reason. Test `candidate_summaries_use_successful_suffix_prefix_trial_counts_and_condition_intersection` covers the A/B asymmetric case and A's under-trialled stage at 2 (`:658-695`).
- **Sol 2 — evidence spans:** `attribute_evidence` (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:353-382`) credits a span only when it is fully contained in exactly one trial; otherwise every overlapping trial is marked unattributable, while no-overlap spans retain the existing fallback. `incomplete_or_unattributable_trials_never_pass` covers spans crossing either edge of one trial and a contained span (`:636-643`).
- **Sol 3 — output preflight:** `metadata_from_env` now calls `validate_output_path` before returning metadata (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:45-88`). It refuses existing destinations and parents that are missing or not directories; `write_record` retains its final `.create_new(true)` guard (`:248`). The hardware entry calls `process_metadata()` before any probe (`crates/ezsdr-radio-uhd/tests/hardware.rs:1467-1470`). `metadata_and_writer_fail_closed` covers an existing file and a missing parent (`measurement.rs:898-908`).
- **Sol 4 / P734 — mutation:** `plan/maintenance/tools/mutations.json:10795-10803` now gives missing values a non-empty `not_applicable` mutant, so the missing-environment assertions demonstrate rejection rather than killing a panic. P734 was killed.
- **O1 — shared builder derivation:** `description_surfaces` centralizes the tree, admission lead and envelope derivation (`crates/ezsdr-radio-uhd/src/provider/mod.rs:66-73`); both `from_binding` and `with_measurement_timing` call it (`:319-320,367-372`). Ordinary construction behavior is unchanged. Mutation rows P706-P709 and P724 were updated for the shared path; J277's source was updated to the helper and remains covered.
- **O5 — spec wording:** `design/18-uhd-radio.md:353` now says “replace the Description's timing with candidate values.”
- **O4:** left unchanged per the adjudication.

### Verification

- `cargo test --workspace`: passed; 1,781 passed, 0 failed across 68 suites. The changed tests are pure and were run once.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problems.
- `git diff --check`: passed. `plan/maintenance/tools/mutations.json` parsed as valid JSON.
- Remote targeted mutation run on `sim02:4,sim03:4`: **16/16 killed** — J277, P706-P710, P716-P717, P724, P728-P731, P734, P742-P743. This includes the condition-edge, trial-count, span-boundary, metadata-preflight, missing-value, and shared-builder checks; no full mutation list was run.
- Repository changes remain unstaged and uncommitted.

### Remaining

- No hardware entry point, hardware test or measurement was run. UHD-feature clippy was not run because `hardware.rs` was not changed in this fix.
- The full mutation lists were not run. Stage 3b-2 sweeps and stage 3c overflow-restart semantics remain deferred.
