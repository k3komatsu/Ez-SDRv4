# 段階 3a：実装役の報告

## 実装（`report-3a.md`）

STATUS: done


### Implemented

- `crates/ezsdr-radio/src/measurement.rs`: added the versioned serde record and supporting condition, value, observation, and typed `GridValue` types. The single-variant schema enum accepts only `ezsdr.radio.measurement.v1`. `rule(key)` defines all capability comparison classes, including concrete numbered channel/stream keys; profile values come from `Description`. `compare` checks source/profile, measured and unmeasured values, exact comparable-key coverage, grid membership, condition/layout consistency, and metadata. Records with no measured capability values are rejected to prevent a vacuous successful comparison.
- `crates/ezsdr-radio/src/lib.rs`: exported the module and registered the record schema for generation. `crates/ezsdr-radio/src/channels.rs` makes `capabilities()` crate-visible so the unit test can check the rule/value key sets against it.
- `crates/ezsdr-mock-radio/src/profile.rs` and `src/lib.rs`: exposed the existing `Profile::terms()` and `BLOCK_LEN` without changing their values, for acceptance comparisons.
- `crates/ezsdr-radio/tests/measurement.rs` and `tests/fixtures/measurement/{valid.json,invalid-comparisons.json}`: added record, rule, coverage, malformed-value, condition, and grid-containment tests with hand-written JSON fixtures.
- `crates/ezsdr-acceptance/tests/measurements.rs` and its `tests/fixtures/measurement/records/valid.json` input: compare each available measurement against descriptions built with MockRadio's and UHD's own terms and block lengths. UHD's `block_len` comes from the selector when present and otherwise defaults to `DEFAULT_BLOCK_LEN`. The absent/empty measurement directory remains a valid zero-record case; a separate fixture-directory test exercises file discovery and loading.
- `schemas/radio/measurement.v1.json`: generated from the Rust type and included in the schema-freeze test.
- `design/07-radio-model.md`: updated RM-20's schema count and added RM-35; `plan/maintenance/tools/mutations.json`: added P675–P694.

No profile values, versions, or `SCHEMA_CHANGELOG.md` entries changed. No measurement suite, candidate-value mechanism, hardware code, or committed hardware record was added.

### Design/spec updates

RM-35 now states the schema identifier and refusal behavior, record contents, all eight comparison classes and their keys, condition and metadata validation, exact coverage rules, the requirement for at least one measured value, and comparison against both Providers' actual descriptions. RM-20 now identifies twelve committed Radio Model schemas. These additions capture gaps found in review: a record must not pass without evidence, and comparison must use the specific Provider's terms and block length. The typed grid representation preserves the full admissible set, rather than reducing grid comparisons to endpoints.

### Tests and gates

Each new test target was run 20 consecutive times, with no failures:

- `ezsdr-radio --lib measurement::tests`: 2 tests; pins capability/rule/value coverage and numbered `c1`/`s1` lookup.
- `ezsdr-radio --test measurement`: 13 tests; pins each comparison class, equal/stricter acceptance and looser rejection, key coverage, schema/source/profile checks, condition and metadata validation, measured-value requirement, and grid containment including interior values and mixed `Step`/`Values` representations.
- `ezsdr-acceptance --test measurements`: 4 tests; pins file loading and both Provider comparisons, the 3 ms lead that fails MockRadio but passes UHD, and selector-provided UHD `block_len`.

Other requested checks passed:

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` — 0 problems
- `python3 plan/maintenance/tools/rules.py --self-test`
- `rm_20_schema_freeze`
- `git diff --check`

The full mutation lists, Python suites, Docker checks, and hardware measurements were not run for this stage; they were not part of the stage 3a brief/gate.

### Mutation results

All new rows were run remotely on sim02/sim03 and ultimately **killed**:

- P675–P681: rule boundaries, grid-step containment, alignment multiple, guarantee strength, truth implication, and equality.
- P682–P686: duplicate, missing, and unknown values; fake source; profile mismatch.
- P687–P691: Provider-specific UHD terms, requiring measured evidence, hardware-time and throughput rule mappings, and selector `block_len`.
- P692–P694: full `Values`-grid membership (including an omitted interior value), `Step` profile versus `Values` measurement, and `Values` profile versus `Step` measurement.

The first P682 run had a mutation compile error; its row was corrected and later killed. P687 had a stale anchor in one rerun (`NOT FOUND`); the anchor was corrected and the row was subsequently killed in a dedicated remote run. No mutation rows remain unverified; this stage intentionally did not run the full mutation lists.

### Self-review rounds

All three read-only rounds used OpenCode's `deepseek-v4.1-flash-free` plan reviewer. Round 3's P1 and other findings were fixed; the brief capped self-review at three rounds, so no fourth post-fix review was run.

1. **Round 1 — P1: 0; P2: 4; P3: 4.** Added the missing stream-rate containment coverage, rejected all-unmeasured records, expanded the RM-35 test list, added invalid JSON fixture coverage, and corrected mutation-file indentation. Also completed class/key mapping tests, second-channel/stream lookup, metadata rejection cases, and stronger assertions for the Provider-specific lead comparison.
2. **Round 2 — P1: 0; P2: 1; P3: 3.** Added explicit `hardware_time` and RX/TX throughput rule assertions; made the UHD description honor selector `block_len` with its existing default; corrected the observation-test diagnostic assertion; and reran both Radio Model unit tests 20 times. The stale P687 mutation anchor was corrected and killed remotely.
3. **Round 3 — P1: 1; P2: 4; P3: 1.** The P1 endpoint-only test gap was fixed with an interior-value omission case. P2 findings were resolved by spelling out the rule/key mapping in RM-35, updating the stale schema-count test-table row, exercising the acceptance file-loading path using a fixture directory, and testing both mixed `Step`/`Values` containment directions. The tautological key-set assertion flagged as P3 was removed; the independent capability/rule/value coverage test remains. P692–P694 were added and killed remotely.

### Remaining

Stage 3a is complete. Stage 3b/3c still need to add and exercise the measurement suite and produce hardware measurements; the repository currently has no measurement records under `crates/ezsdr-radio/measurements/`. No real hardware behavior was measured or validated in this stage.


## 修正 1 回目（`fix-3a-report.md`）

STATUS: done


### Adjudicated findings

- **Sol 1 — rounded integer comparisons:** `measurement.rs:519,589,644-652` now refuses numeric operands and grid members/endpoints/steps with absolute magnitude `>= 2^53`. Regressions cover `repeat_max_samples` at `2^53`/`2^53+1` and an `Integer` grid ending at `2^53+1` (`tests/measurement.rs:266,276`). P695 and P696 were killed.
- **Sol 2 — Provider verdicts in the file path:** `acceptance/tests/measurements.rs:18-50,66-82` now returns each file's two verdicts; the acceptance gate asserts both are clear. The fixture directory has a Mock-only rejection (`mock-only-rejects.json`), a UHD-only rejection (`uhd-only-rejects.json`), and a passing record. The file-path test asserts both rejection directions and the passing record. P698 and P699 were killed; P687 was updated for the default block length and killed.
- **Sol 3 — singleton grid containment:** `measurement.rs:631-633` checks the sole member before testing step ratios. Tests cover the `{6}` subset case and Integer-profile comparisons against Values, Integer and Step grids (`tests/measurement.rs:229-264`). P697 was killed.
- **Sol 4 — observation-only records:** removed the minimum-measured-value rejection. A record with every comparable value explicitly unmeasured and an observation now passes (`measurement.rs:412-460`, `tests/measurement.rs:351-360`). RM-35 was corrected (`design/07-radio-model.md:343-344`); obsolete P688 and its rejection test were removed, without reusing the ID.
- **O1 — UHD `block_len`:** acceptance uses `DEFAULT_BLOCK_LEN` (`acceptance/tests/measurements.rs:7-15`); removed the selector helper, its test and P691. `radio.rx.block_len` remains un-compared.

### Self-review findings

One read-only round used `deepseek-v4.1-flash-free#max` (`review-3a-4.log`): **P1: 0**. No second round was run, as instructed.

- **P2-1, adopted:** added asymmetric RX/TX stream and channel bounds plus a cross-direction `concurrent` reference case (`tests/measurement.rs:454-500`). P700 and P701 were killed.
- **P3-1, adopted:** documented MockRadio `BLOCK_LEN` and UHD `DEFAULT_BLOCK_LEN` in RM-35 and design note 41 §4 (`design/07-radio-model.md:343`, `plan/phase8/41-measurement-records.md:49`).
- **P3-2, adopted:** duplicate channels in one stream now stop before the cross-stream check (`measurement.rs:495-503`); the test verifies there is no misleading cross-stream violation (`tests/measurement.rs:427-432`). P702 was killed.
- **P3-3, adopted:** aligned the P675–P702 mutation entries' indentation with the surrounding file; P688 and P691 remain unused IDs.

### Checks

- `cargo test -p ezsdr-radio --test measurement`: 18 passed; repeated 20 times after the final test/code changes, all passed.
- `cargo test -p ezsdr-acceptance --test measurements`: 3 passed; repeated 20 times, all passed. The final workspace run also included this target.
- `cargo test --workspace --no-fail-fast`: passed after the final changes. An earlier run had one timing-sensitive UHD unit test fail; that test passed alone, and subsequent full workspace runs passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problems.
- `cargo test -p ezsdr-radio --test radio_model rm_20_schema_freeze`: passed.
- `git diff --check`: passed.
- Mutation anchors: all 1,054 active rows match exactly once. Remote mutations P677, P687, P695–P699 and P700–P702 were all killed (10/10). The full mutation lists were not run.

### Remaining

- No hardware measurement or Python/Docker suite was run; these are outside this fix brief. No measurement records were added to the runtime `measurements/` directory.
- Full mutation lists remain for the orchestrator's gate. Changes are uncommitted and unpushed.


## 修正 2 回目（`fix2-3a-report.md`）

STATUS: done


### Findings

- **Sol 1 — acceptance Provider gates:** `crates/ezsdr-acceptance/tests/measurements.rs:53-68` now uses one result-returning comparison gate for the runtime measurements directory and the fixtures. The Mock-only and UHD-only rejecting fixtures each assert that the same gate rejects for the expected Provider; the passing fixture still asserts success (`:76-99`). The existing runtime-directory test continues through that gate (`:70-73`). P698/P699 and new P703/P704 were killed.
- **Sol 2 — asymmetric RX/TX bounds:** `crates/ezsdr-radio/tests/measurement.rs:471-485` now verifies a complete record with RX stream/channel 1 passes for a two-RX/one-TX description; additional comparable capabilities are explicitly unmeasured. Existing TX out-of-range cases remain. P700/P701 and new P705 were killed.
- **Orchestrator — wall-clock-sensitive UR-21 test:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:2906-2948` now uses the manual FakeDevice clock and `Frozen`/`device_at`. It advances A deterministically, then checks B/C reservation and the refusal at an unchanged Core and device time. It asserts C remains held at its start, the `UR-21` rejection, `TIME_ERROR { refused }`, and `COMMAND_REJECTED`. The test passed 50/50; phase7 mutation C03 was killed, including a rerun after the final test adjustment.

### Checks

- Acceptance measurements test binary: 20/20 runs passed.
- RX direction/bounds test: 20/20 runs passed.
- `ur_21_a_moved_start_on_a_held_burst_s_is_refused`: 50/50 runs passed.
- `cargo test --workspace`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problems.
- `git diff --check`: passed.
- `cargo test -p ezsdr-radio --test radio_model rm_20_schema_freeze`: passed.
- Targeted remote mutations: P698, P699, P700, P701, P703, P704, P705 and C03 all killed (maintenance 7/7; phase7 1/1).

### Remaining

No open issue from these findings. Changes remain uncommitted and unpushed.

