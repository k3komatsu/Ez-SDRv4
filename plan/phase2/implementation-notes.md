# Phase 2 implementation notes

## Step 1

- [x] Confirmed the Kernel base matches `96976c5` and the working tree was clean before applying the patch.
- [x] `git apply plan/phase2/patches/01-kernel-amendments.patch` succeeded; exactly the specified 28 files changed.
- [x] `cargo +1.85.0 test --workspace`: 361 passed, 0 failed.
- [x] `cargo +stable test --workspace`: 361 passed, 0 failed.
- [x] `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed.
- [x] Read the Kernel diff; no files outside the patch were edited.
- Mutation checks: none required; spec 06 §2 records that the patch tests were mutation-checked when the patch was produced.
- Suggested commit subject: `feat(kernel): apply Phase 2 amendments`

## Step 2

- [x] Applied every KA amendment in spec 06 §2 to the named accepted specs and Phase 1 overview, including replacement text, new rules, status rows, and marker updates.
- [x] Added the named Step 1 tests to the corresponding spec test tables; replaced the obsolete SC-8 test row and updated the SB-22h cases.
- [x] This step's edits were limited to `design/01-time-model.md` through `design/05-module-api.md` and `plan/phase1/00-overview.md`. The cumulative diff also contains the Step 1 patch.
- [x] Ran the prescribed link check and opened the actual relative-link targets from their containing files; all resolve. Its `CHECK` output also includes two Rustdoc type links in the patch and a quoted shell-command fragment in the implementation plan, which are grep false positives rather than Markdown file links.
- [x] `cargo +1.85.0 test --workspace`: 361 passed, 0 failed.
- [x] `cargo +stable test --workspace`: 361 passed, 0 failed.
- [x] `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed.
- Mutation checks: none; this step changed documents only.
- Suggested commit subject: `docs: record Phase 2 Kernel amendments in accepted specs`

## Step 3

- [x] Added `tests/support/run_doubles.rs` with `Probe`, `SimAuthority`, `SteppedProvider`, `RecordingSink`, `ProbeExecutor`, `TestLinkModule`, and the four shared fixture functions from spec 06 §3.
- [x] Added the `TestProvider` builders and `TestLimitsCheck` gate behavior; left `tests/spec_binding.rs` unchanged.
- [x] Added `ma_44_the_run_doubles_record_their_calls`; the exact probe sequence passed.
- [x] `cargo +1.85.0 test --workspace`: 362 passed, 0 failed.
- [x] `cargo +stable test --workspace`: 362 passed, 0 failed.
- [x] `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed.
- Mutation checks: none named in this step's done-list.
- Suggested commit subject: `test(kernel): add coordinator test doubles`

## Step 4

- [x] Added the coordinator public API and the Step 4 state types, with only the ten public items specified by spec 06 §4 allow-listed.
- [x] Implemented Run entry, KC-4 Provider grouping and extra-object refusals, `validate()`, Policy compilation, `plan()`, Simulation-class refusal, and fragment-to-instance routing.
- [x] Implemented the failure end path, cleanup steps 0, 1, 5 and 8, and the Step 4 Manifest fields, including verbatim input documents, failure reason, transitions, Provider fidelity and sections, and sealing.
- [x] Added all eight Step 4 coordinator tests; `kernel_surface` passes.
- [x] `cargo +1.85.0 test --workspace`: 370 passed, 0 failed.
- [x] `cargo +stable test --workspace`: 370 passed, 0 failed.
- [x] `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed.
- mutation: KC-2 / `kc_02_a_wall_paced_authority_is_refused`: fails when disabled — yes.
- mutation: KC-4 duplicate description / `kc_04_two_objects_for_one_description_are_refused`: fails when disabled — yes.
- mutation: KC-4 unbound supplied name / `kc_04_an_object_under_no_resource_is_refused`: fails when disabled — yes.
- Suggested commit subject: `feat(kernel): add coordinator entry and failure manifests`

## Step 5

- [x] Implemented input verification and Manifest refs, Link creation/attachment, event collection, prepare, arm, T0 and start in the pipeline.
- [x] The Step 5 tests and all preceding coordinator tests pass; the complete coordinator suite now has 65 tests.
- mutation: KC-9 missing bytes / `kc_09_an_input_must_be_supplied_and_match_its_hash`: fails when disabled — yes.
- mutation: KC-9 hash / `kc_09_an_input_must_be_supplied_and_match_its_hash`: fails when disabled — yes.
- mutation: KC-9 size / `kc_09_an_input_must_be_supplied_and_match_its_hash`: fails when disabled — yes.
- mutation: KC-9 URI scheme / `kc_09_an_input_must_be_supplied_and_match_its_hash`: fails when disabled — yes.
- mutation: KC-10 Link descriptor / `kc_10_link_descriptor_must_equal_the_registered_one`: fails when disabled — yes.
- mutation: KC-12 stop at first prepare failure / `kc_12_a_prepare_failure_stops_the_loop_and_cleans_up_what_was_prepared`: fails when disabled — yes.
- mutation: KC-13 arm order / `kc_13_arm_and_start_follow_instance_order_cleanup_reverses_it`: fails when disabled — yes.

## Step 6

- [x] Implemented schedule resolution, cumulative admission of timed updates, the stepping loop, orderly drain, and step/event handling.
- [x] All thirteen Step 6 test rows pass; the full coordinator suite passes.
- mutation: KC-17 working copy / `kc_17_scheduled_updates_are_admitted_cumulatively`: fails when disabled — yes.
- mutation: KC-16 ambiguity / `kc_16_an_ambiguous_spec_time_is_refused`: fails when disabled — yes.
- mutation: KC-16 off-root / `kc_16_off_root_negative_and_overflowing_times_are_refused`: fails when disabled — yes.
- mutation: KC-16 negative offset / `kc_16_off_root_negative_and_overflowing_times_are_refused`: fails when disabled — yes.
- mutation: KC-16 overflow / `kc_16_off_root_negative_and_overflowing_times_are_refused`: fails when disabled — yes.
- mutation: KC-19 lead / `kc_19_a_reject_at_plan_burst_with_a_short_lead_is_refused`: fails when disabled — yes.
- mutation: KC-22 downgrade / `kc_22_a_downgraded_livelock_still_ends_the_run`: fails when disabled — yes.
- mutation: KC-22 cap / `kc_22_a_same_instant_wakeup_loop_is_step_livelock`: fails when disabled — hangs as expected; confirmed by a 30-second timeout.
- mutation: KA-12 drain / `kc_39_orderly_cleanup_drains_the_tail`: fails when disabled — yes.

## Step 7

- [x] Implemented Action admission and dispatch, Provider coercion, scheduled dispatch, Session submissions, waveform ingestion, Lease handling and child-Run refusal.
- [x] All twenty-two Step 7 test rows pass; the full coordinator suite passes.
- mutation: KC-24 frozen dispatch / `kc_24_a_module_action_during_cleanup_is_refused`: fails when disabled — yes.
- mutation: KC-24 Module Stop / `kc_24_a_module_stop_without_target_is_refused`: fails when disabled — yes.
- mutation: KC-24 non-Provider target / `kc_24_a_burst_to_a_non_provider_target_is_refused`: fails when disabled — yes.
- mutation: KC-24 SC-23b unrelated clock / `kc_24_a_burst_time_on_an_unrelated_root_is_refused`: fails when disabled — yes.
- mutation: KC-24 missing TX SampleClock attribution / `kc_24_a_burst_needs_a_transmit_sample_clock`: fails when disabled — yes.
- mutation: KC-24 Module `RejectAtPlan` / `kc_24_a_module_reject_at_plan_burst_is_refused`: fails when disabled — yes.
- mutation: KC-26 applied-nothing / `kc_26_a_provider_that_applies_nothing_is_refused`: fails when disabled — yes.
- mutation: KA-6 Provider coercion / `rs_17_a_session_rate_change_is_coerced_by_its_provider`: fails when disabled — yes.
- mutation: KC-28 pre-admission `check_entry` / `kc_28_a_malformed_action_takes_no_sequence_number`: fails when disabled — yes; the test also asserts that rejected malformed input was not ingested.

## Step 8

- [x] Implemented end-request escalation and recording, final event reaction, artifact marks, cleanup failures, the wedged-slot Manifest path and the remaining Manifest fields.
- [x] All 65 test names in spec 06 §12 exactly match `tests/coordinator.rs`; all pass.
- mutation: KC-30 validate containment / `kc_30_a_panic_in_coerce_fails_validate`: fails when disabled — yes.
- mutation: KC-30 poisoned-slot handling / `kc_30_a_panic_in_coerce_fails_validate`: fails when disabled — yes.
- mutation: KC-31 artifact marks / `kc_31_mark_artifact_marks_only_artifacts_open_then`: fails when disabled — yes.
- mutation: KC-32 escalation record / `kc_32_an_abort_during_orderly_escalates_and_is_recorded`: fails when disabled — yes.
- mutation: KC-41 Sink stop error / `kc_41_a_failing_sink_stop_is_a_cleanup_failure`: fails when disabled — yes.
- mutation: KC-44 nonblocking Manifest slot / `kc_44_a_wedged_step_does_not_prevent_the_manifest`: hangs as expected; confirmed by a 60-second timeout.
- [x] `kernel_surface` passes; OV-23b reports **115 NEW / 291 public items**.
- [x] `cargo +1.85.0 test --workspace`: 427 passed, 0 failed.
- [x] `cargo +stable test --workspace`: 427 passed, 0 failed.
- [x] `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed.
- [x] `git diff --check`: passed.
- Step 8 is complete. Stop for Review K; Step 9 has not started.

## Fixes after Review K

- **B1 [P0] — KA-6 / KC-24:** Provider coercion now runs only for Session and schedule origins. A Module-origin `UpdateParameter` is admitted without calling `Provider::coerce`; the regression test asserts the Module submit succeeds and the Provider's coercion count does not increase.
- **B2 [P1] — KA-12:** every later `StopRx` cleanup call sets `closing` once the drain has started. The abandoned drain owner checks `closing` after it wakes and returns before stopping its own instance. The regression test releases an over-deadline step, waits for the abandoned cleanup call to settle, and asserts the Sink is not stopped afterward.
- **N1 [P1] — KC-30:** Link `descriptor()` and Authority TimeAuthority `now`, `schedule`, and `cancel` are contained. Invalid `now()` results and panics become the active-stage failure; a cancel panic becomes a `FreezeDispatch` cleanup failure. Added panic regressions for the Link descriptor and all three time-handle calls.
- **N2 [P1] — KC-45 / KC-2:** install the built Routing before rejecting a non-Simulation plan, preserving its plan and `execution_class` in the Manifest. The WallPaced refusal test asserts both fields.
- **N3 [P1] — KC-8:** register Vocabulary event pairs per Executor Island and keep `DEVICE_LOST` on the first Island source for that instance. The two-Island test emits from the second Island and checks its own counter.
- **N4 [P2]:** store the validation `AdmissionResult` before checking admission; the refusal test checks its rejected entries are present in the Manifest.
- **N5 [P2] — RS-3:** stop the lifecycle pipeline after a pending end request at prepare, arm and start boundaries. The prepare-time abort test includes a later Sink fragment and asserts it is not prepared, armed or started.
- **N6 [P2] — KA-13:** emit the invalid-namespace cleanup failure only when the Provider supplies sections. Added a no-sections regression.
- **N7 [P2] — KC-36:** `finish()` checks the Lease before requesting the client stop cause. Added an expired Detached Lease regression.
- **N8 [P2] — KC-19:** an `Err` from `compare_durations` now fails Stage Arm with the KC-19 reason. No valid coordinator input reaches this error branch: the primary root is used by the validated Run pipeline, `min_command_lead` is restricted to non-negative `host.monotonic`, and registered rates are capped. An attempted over-cap root test is rejected by `ManualTimeAuthority` before reaching the comparison; no test seam was added solely to manufacture an invalid internal operand.
- **N9 [P2]:** removed the stale dead-code allowances, unused bindings, discarded target/clone work and duplicate timing helper; schedule resolution uses `ActionTemplate::is_timed()`.
- **N10 [P2] — KC-28:** Session admission now accumulates violations from all compiled Actions against a working configuration updated only by admitted Actions, then dispatches none if any violation exists. Current `session::compile` branches each append at most one Action, so a multi-Action refusal has no reachable fixture input today.
- **N11 [P2] — KA-18:** missing link drop counts serialize as JSON `null`; the `drops()` panic regression checks both the null value and the cleanup failure.
- **Mutation checks (13):** B1 / `kc_24_a_module_update_is_not_coerced_by_the_kernel`; B2 / `ka_12_an_abandoned_drain_does_not_stop_a_sink_after_cleanup`; N1 / `kc_30_a_panicking_link_descriptor_fails_plan_without_unwinding`, `kc_30_a_panicking_time_now_fails_validate_without_unwinding`, `kc_30_a_panicking_time_schedule_fails_arm_without_unwinding`, `kc_30_a_panicking_time_cancel_is_a_cleanup_failure`; N2 / `kc_02_a_wall_paced_authority_is_refused`; N3 / `kc_08_an_executor_event_from_a_second_island_has_its_own_counter`; N4 / `kc_45_a_rejected_admission_is_kept_in_the_manifest`; N5 / `kc_24_a_prepare_abort_stops_before_arm_and_start`; N6 / `ka_13_an_invalid_provider_namespace_without_sections_is_not_a_failure`; N7 / `kc_36_finish_checks_an_expired_detached_lease`; N11 / `ka_18_a_failed_link_drop_snapshot_is_unknown_not_zero`. Each test failed with its corresponding guard disabled and passed after restoring it. N8's comparison-error branch is unreachable from valid inputs as described above; N9 is cleanup-only and N10 has no current multi-Action input.
- **Full verification:** `cargo +1.85.0 test --workspace`: 439 passed, 0 failed; `cargo +stable test --workspace`: 439 passed, 0 failed; `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed; `git diff --check`: passed.
- Suggested commit subject: `fix(coordinator): address Review K findings`
- Review K findings are addressed or recorded as an accepted risk. Step 9 has not started.

### Review K re-review 1

- **R1 [P2] — KA-12:** `StopRx` and `RestoreBaseline` now coordinate through the per-instance `done` lock across slot acquisition and Module calls. If an abandoned drain leaves a Sink or Executor without StopRx, RestoreBaseline performs StopRx before cleanup when the slot is available; if the slot is still held, it records KC-39, skips cleanup, and leaves the RestoreBaseline marker so the stale drain skips the instance after it wakes. `drain_cleanup` rechecks abort/closing after a blocked `next_wakeup` returns. Added coverage for both a slot held by the drain and an available slot after a blocked Authority call.
- **R2 [P2] — KC-30 / RS-3:** check the pending end after setting T0, after resolving the schedule, and after admitting it, before any Module starts. The nth-call `now()` double panics during Arm and the regression asserts Stage Arm failure with no Provider `start`.
- **R3 [P2] — documented risk accepted:** partial successful PrepareReports remain absent from the Manifest after a mid-prepare Abort. KC-12 does not require incomplete reports to be recorded, and `collect_prepare` rejects an absent fragment report; placing unvalidated partial data in `MergedPrepare` would imply a completed merge. No schema change was made for this optional record detail.
- **R4 [P2]:** the KA-12 regressions use ConditionVariable-backed observations and bounded event waits instead of polling. They assert StopRx/cleanup ordering, call counts, and that an abandoned drain cannot step or stop a Module after cleanup. The Module-origin update regression snapshots `Provider::coerce` immediately before Executor submission, checks the count is unchanged, and asserts the Provider received the Action.
- **Mutation checks (4):** R1 / `ka_12_an_abandoned_wakeup_drain_stops_the_sink_before_cleanup`: disabling the RestoreBaseline StopRx fallback fails because Sink stop is missing; disabling both the after-wakeup check and RestoreBaseline completion marker fails because a Module is stepped after cleanup. R2 / `kc_30_a_panicking_time_now_during_arm_does_not_start_modules`: disabling the pre-start end checks fails because Provider `start` is called. R4 / `kc_24_a_module_update_is_not_coerced_by_the_kernel`: attempting Module-origin coercion with a nonblocking Provider lock fails because the Action is refused. Each mutation failed its test; the changes were restored.
- **Full verification:** `cargo +1.85.0 test --workspace`: 441 passed, 0 failed; `cargo +stable test --workspace`: 441 passed, 0 failed; `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed; coordinator suite: 79 passed; `git diff --check`: passed. The existing kernel surface test continues to report 115 NEW / 291 public items.
- Suggested commit subject: `fix(coordinator): close Review K re-review gaps`
- Review K follow-up is complete. Step 9 has not started.

### Review K re-review 2

- **B3 [P0] — KA-12 / RS-8a:** `done` now protects only marker checks/updates and nonblocking per-instance slot acquisition. `StopRx` and `RestoreBaseline` release it before calling a Module while retaining that instance's slot; `StopTx` releases it before `Provider::stop`. Cleanup rounds recheck the step-5 marker after obtaining a slot. A wedged Module therefore holds its own slot without blocking cleanup for other instances.
- **Regression test:** `kc_44_a_wedged_provider_stop_does_not_block_other_cleanup` wedges one Provider in `stop`, then asserts the drain steps a healthy Provider before the Sink stops, both healthy instances are cleaned up, the Sink artifact reaches the Manifest, the wedged Provider records a `KC-39` RestoreBaseline failure, and total cleanup stays below two default cleanup deadlines. The test completes in about one default cleanup deadline.
- **Nonblocking notes recorded:** the Arm `now()` panic fixture still selects call 6, so a future refactor of Authority calls could make it target a different phase. If the RestoreBaseline fallback Sink stop itself fails, the failure appears under step 5 because that fallback runs there; KC-41 still requires the fragment to be named.
- **Verification after B3:** coordinator integration suite: 80 passed; `cargo +stable test --workspace` and `cargo +1.85.0 test --workspace` passed; `cargo +stable clippy --workspace --all-targets -- -D warnings` passed. The first stable workspace run exposed the existing OV-23a marker being moved by rustfmt; the marker was restored and the workspace rerun passed.
- Suggested commit subject: `fix(coordinator): avoid cross-instance cleanup blocking`
- Step 9 has not started.

### Review K sticky re-review 3 follow-up

- **Opus verdict:** `PASS_WITH_RISK`; B3 is closed and no blockers remain.
- **P2 lock-order note:** moved `current_stop_mode()` before acquiring `done` in StopRx. No `done` → `end` lock edge remains there.
- **Drain test gap:** the B3 regression now schedules a wakeup for the healthy Provider and asserts `healthy:step:1` precedes `rec:stop:Orderly`, covering step 3's drain while the wedged Provider holds its own slot.
- **Accepted residuals:** the `now()` panic fixture still targets call 6; the marker check/acquire race has no deterministic test seam; other previously identified test gaps remain nonblocking. The review also confirmed the per-instance marker recheck is needed to prevent stepping a cleaned-up instance.
- **Verification after follow-up:** `kc_44_a_wedged_provider_stop_does_not_block_other_cleanup` passed with the drain wakeup; coordinator suite: 80 passed; `cargo +stable test --workspace` and `cargo +1.85.0 test --workspace` passed; `cargo +stable clippy --workspace --all-targets -- -D warnings` passed.
- **Formatting note:** `cargo +stable fmt --all` was inadvertently run during the B3 work, contrary to the implementation-plan instruction. Files that were clean before the command were restored; the cited forwarding methods and regression assertion were manually returned to the compact style. Some already-modified files may still contain formatter-only changes. No formatter was run again.

## Step 9

- [x] Recorded `baseline_external_packages.txt` before updating `Cargo.lock`; it contains 27 external packages and does not name `ezsdr-kernel`.
- [x] Added all nine Phase 2 crates as workspace members with their specified package metadata, allowed dependencies, dev-dependencies, crate documentation and safety lints. The acceptance crate is `publish = false` and contains the PO-4 baseline.
- [x] `cargo +stable build --workspace` succeeded. `Cargo.lock` adds exactly nine local workspace packages and their dependency lists; the 27 external packages are unchanged, and none of the new entries has a `source`.
- [x] `cargo +1.85.0 test --workspace`: 442 passed, 0 failed.
- [x] `cargo +stable test --workspace`: 442 passed, 0 failed.
- [x] `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed.
- Mutation checks: none required by this step.
- Suggested commit subject: `build(workspace): add Phase 2 crate skeletons`

## Step 10

- [x] Implemented `ezsdr-sim`: the `sim` Vocabulary, `sim.seed` and `sim.faults` readers/checks, SplitMix64 `SimRng`, virtual-time constants, and SE-12 schema generation.
- [x] Implemented `ezsdr-sim-engine`: the descriptor, free-running root and Authority, time conversion and scheduling, callback ordering/cap, and wakeups.
- [x] Added the six Vocabulary tests and seven Engine tests from spec 08 §5; both crate suites pass.
- [x] Generated `schemas/sim/fault_entry.v1.json` and `schemas/sim/seed.v1.json`; added the SE-12 line to `schemas/SCHEMA_CHANGELOG.md`. Kernel `schema_freeze`: 4 passed.
- Mutation: SE-5 unknown fault target / `se_05_checks`: failed with the target refusal disabled; passed after restoration.
- Mutation: SE-10 `InPast` / `se_10_time_authority_contract`: failed with the past-time refusal disabled; passed after restoration.
- Mutation: SE-11 callback cap / `se_11_next_wakeup_order_ties_and_cap`: with `CALLBACK_CAP = usize::MAX`, the test timed out after 60 seconds as expected; passed after restoring the cap to 1000. The system `timeout` command was unavailable, so the same bound was enforced with Python's standard-library subprocess timeout and process-group cleanup.
- [x] `cargo +1.85.0 test --workspace`: 455 passed, 0 failed.
- [x] `cargo +stable test --workspace`: 455 passed, 0 failed.
- [x] `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed.
- Suggested commit subject: `feat(sim): add Simulation Vocabulary and Engine`

## Step 11

- [x] Implemented the `radio` Vocabulary descriptor and ordered registration for all 29 RM-4 keys, nine RM-10 event kinds, RM-12 verbs and `RfEnvelopeCheck`.
- [x] Added the RM-19 RF safety check for merged effective/proposed configurations, including bands, gain, channel enables, both antenna directions and malformed profile sections.
- [x] Added the RM-20 envelopes, RM-22 payload types and ten spec 07 §6 integration tests.
- [x] Generated all seven `schemas/radio/*.v1.json` files and added the RM-20/RM-22 changelog entry.
- Mutation: RM-19 frequency band / `rm_19_rf_envelope_cases`: failed with the band guard disabled; passed after restoration.
- Mutation: RM-19 maximum gain / `rm_19_rf_envelope_cases`: failed with the gain guard disabled; passed after restoration.
- Mutation: RM-19 disabled TX channel / `rm_19_rf_envelope_cases`: failed with channel-enable validation disabled; passed after restoration.
- Mutation: RM-19 TX antenna / `rm_19_rf_envelope_cases`: failed with the TX antenna guard disabled; passed after restoration.
- Mutation: RM-19 RX antenna / `rm_19_rf_envelope_cases`: failed with the RX antenna guard disabled; passed after restoration.
- Mutation: RM-19 malformed section / `rm_19_a_malformed_section_is_one_violation`: failed with parse refusal disabled; passed after restoration.
- [x] `cargo +1.85.0 test --workspace`: 465 passed, 0 failed.
- [x] `cargo +stable test --workspace`: 465 passed, 0 failed.
- [x] `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed.
- [x] `git diff --check`: passed.
- Suggested commit subject: `feat(radio): add Radio Model Vocabulary`

## Step 12

- [x] Implemented `ezsdr-hostmem`: host memory identity, reusable Arc-backed slots, planar `cf32` helpers and panic-free interleaving for invalid ranges or absent host bytes.
- [x] Implemented `ezsdr-link-host`: Module and Link descriptors, zero-capacity refusal, all three back-pressure policies, persistent drop counts and cleared-on-read `DropCarry`.
- [x] Implemented the `sink` Vocabulary, its `capture` action compilation and HD-14 rejected-request payload schema.
- [x] Added the two host memory tests, two Link tests and two Sink Vocabulary/schema tests from spec 10 §7.
- [x] Generated `schemas/sink/request_rejected_payload.v1.json` and added the HD-14 changelog entry.
- Mutation: HD-4 capacity-zero refusal / `hd_04_descriptor_and_create`: failed when the guard was disabled; passed after restoration.
- Mutation: HD-5 Block policy / `hd_05_policies`: failed when a full queue recorded a drop under `Block`; passed after restoration.
- [x] `cargo +1.85.0 test --workspace`: 471 passed, 0 failed.
- [x] `cargo +stable test --workspace`: 471 passed, 0 failed.
- [x] `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed.
- [x] `git diff --check`: passed.
- Suggested commit subject: `feat(data-path): add host memory link and sink vocabulary`

## Step 13

- [x] Implemented `ezsdr-sink-capture`: binding and descriptor validation, ordered own/request captures, sample-boundary time conversion, interleaved host-memory output, artifact hashes, continuity maps across gaps and SampleClock changes, DropCarry attribution, rejected-request events and partial stop artifacts.
- [x] Added all eleven spec 10 §7 capture-table tests plus `hd_10_unknown_contract_is_rejected`; all twelve pass. Test temp directories are removed after each test.
- Mutation: HD-9 exactly-one-link refusal / `hd_09_prepare_cases`: failed with the guard disabled; passed after restoration.
- Mutation: HD-14 unexpected Action event / `hd_11_an_unexpected_action_is_rejected`: failed with the event suppressed; passed after restoration.
- Mutation: HD-10 unknown-contract refusal / `hd_10_unknown_contract_is_rejected`: failed with unknown contracts accepted; passed after restoration.
- Mutation: HD-14 `N < 1` event / `hd_14_a_bad_capture_value_is_an_event_not_a_failure`: failed when zero was admitted; passed after restoration.
- Mutation: SC-30b carry pass / `hd_10_the_carry_attributes_a_dropped_overflow`: failed when the carry was discarded; passed after restoration.
- [x] `cargo +1.85.0 test --workspace`: 483 passed, 0 failed.
- [x] `cargo +stable test --workspace`: 483 passed, 0 failed.
- [x] `cargo +stable clippy --workspace --all-targets -- -D warnings`: passed.
- [x] `git diff --check`: passed.
- Suggested commit subject: `feat(data-path): add capture Sink Module`

## Step 14

- [x] Completed `ezsdr-mock-radio`: X310-like and ideal profiles, capability tree, coercion and PerformanceEnvelope enforcement, integer clock conversion, RX/TX streams, timed and cold updates, fault handling, burst tracking, stop behavior and the public `DeviceModel`.
- [x] Extended `mr_06_coerce_cases` to assert that 7 GHz is rejected rather than clamped. Extended `mr_07_prepare_cases` to cover second-prepare refusal and rejection of every execution class other than Simulation; added the missing MR-7 execution-class check. Strengthened `mr_16_burst_refusals` to identify the metadata refusal and `mr_17_late_policy_outcomes` to assert that dropped bursts are recorded as rejected.
- [x] All 30 `mock_radio.rs` integration tests pass; the test set matches spec 09 §6. `cargo tree -p ezsdr-mock-radio --edges normal,dev` shows only the allowed local dependencies (`ezsdr-kernel`, `ezsdr-radio`, `ezsdr-sim`, `ezsdr-hostmem`) plus serde packages.
- Mutation: MR-18 application-time validation / `mr_18_a_scheduled_pair_is_checked_when_it_applies`: moving the envelope check to receipt made the test fail; passed after restoration.
- Mutation: MR-2 unknown selector / `mr_02_from_binding_refusals`: allowing `unknown` made the test fail; passed after restoration.
- Mutation: MR-6 out-of-range 7 GHz / `mr_06_coerce_cases`: widening the profile limit made the test fail; passed after restoration.
- Mutation: RM-7 PerformanceEnvelope / `mr_06_coerce_cases`: disabling the envelope check made the test fail; passed after restoration.
- Mutation: MR-7 second prepare / `mr_07_prepare_cases`: disabling the guard made the test fail; passed after restoration.
- Mutation: MR-7 execution class / `mr_07_prepare_cases`: disabling the Simulation-class check made the test fail; passed after restoration.
- Mutation: MR-11 early start / `mr_11_start_cases`: disabling the early-start refusal made the test fail; passed after restoration.
- Mutation: MR-16 metadata / `mr_16_burst_refusals`: accepting non-empty metadata made the test fail; passed after restoration.
- Mutation: MR-17 Drop / `mr_17_late_policy_outcomes`: transmitting a dropped burst made the test fail; passed after restoration.
- Mutation: MR-18 queue full / `mr_18_hardware_timed_updates`: disabling the queue-depth refusal made the test fail; passed after restoration.
- Mutation: MR-21 lost count / `mr_21_overrun_shape`: setting the lost count to `None` made the test fail; passed after restoration.
- Mutation: MR-24 duplicate start / `mr_24_a_bypassing_provider_gets_time_error`: accepting a second `START_OF_BURST` made the test fail; passed after restoration.

## Step 15

- [x] Added the four `v61_*` tests for continuous repeat, capture at a requested sample index, timed TX/RX start and PPS dependency ordering with aligned stream origins.
- [x] Added all four governance carriers: PO-2 crate lints, PO-4 external lockfile baseline, MA-3 Module dependency metadata and PO-11 nondeterminism source scan. The four governance tests and all 19 `v58_*` tests pass.
- Mutation: `v58_10_experiments_name_no_mock_type`: adding a `mock` comment to `experiments.rs` made the test fail; passed after restoration.
- Mutation: `ma_03_no_module_crate_depends_on_another`: adding `ezsdr-link-host` as a MockRadio dev-dependency made the test fail; passed after restoration.
- [x] Required verification after Steps 14–15: `cargo +1.85.0 test --workspace` — 541 passed, 0 failed; `cargo +stable test --workspace` — 541 passed, 0 failed; `cargo +stable clippy --workspace --all-targets -- -D warnings` — passed; `git diff --check` — passed.
- [x] Step 15 completed; Review M and its follow-up fixes are recorded below.

## Review M — owner-directed Opus review and fixes

- Claude Opus 5.5 reviewed the Phase 2 Modules and acceptance tests in one persistent Review M session. The initial review found 3 P0, 6 P1 and 16 P2; successive same-session re-reviews reduced the open findings to P2-a/b, then P2-c/d. The latest Opus verdict was `PASS_WITH_RISK`, with 0 P0, 0 P1 and 2 P2. The reviewer was static-only and ran no commands.
- Closed R1 (Capture Sink Stop target), N7 (RX fault after RX stop), N8 (restore the accepted determinism projection text), the PO-8 dev-dependency check and the MR-27 pre-start timestamp semantics. The review found no regressions in those fixes.
- P2-a: clamp an RX overflow restart sample to the exclusive stream end. The tail regression checks that a 1.5 ms overflow during a 1 ms orderly stop tail reports 500 lost samples, publishes 1,500 samples, and emits the same 500 in the event payload.
- P2-b: convert `FaultEntry.at_ns` with `ns_to_v` at prepare and add T0 only on successful start. The pre-start regression uses a 2 GHz root and checks that 5 ms is recorded as 10,000,000 root ticks.
- P2-c: `schedule_wakeup` excludes fault offsets before start. The MR-14 regression steps a prepared Provider before start and checks `next_due()` stays empty.
- P2-d: calculate absolute fault ticks locally, then commit them after RX SampleClock registration succeeds. A scheduling failure also restores the pre-start tick offsets and start state. The MR-20 regression forces RX registration to fail after arming and checks the pending fault is still recorded as the 10,000,000-tick T0-relative offset.
- No further Opus re-review was run after P2-c/d: all three skill axes were low. These are local MockRadio lifecycle changes outside the Kernel's normal pre-start step path; each has a focused regression, and mutation checks below show each assertion detects its corresponding defect.
- Scratch checks in `/private/tmp/ezsdr-phase2-review-m-20260925`: `cargo +1.85.0 test --workspace` — 546 passed; `cargo +stable test --workspace` — 546 passed; `cargo +stable clippy --workspace --all-targets -- -D warnings` — passed; `git diff --check` — passed. `v58_03_same_seed_same_manifest_projection` passed twice; `v58_02_ten_virtual_seconds_run_faster_than_wall_clock` passed in 0.06 s test time (0.83 s Cargo wall time).
- Mutation checks, each restored afterward: removing the P2-a clamp failed at `lost` 50,000 vs 500; bypassing P2-b's `ns_to_v` failed at 5,000,000 vs 10,000,000; removing P2-c's `started` gate scheduled a spurious wakeup at 5,000,000; committing P2-d's rebased tick before RX registration failed at 5,010,000,000 vs 10,000,000.
- Remaining nonblocking test gap from the static review: there is no acceptance-level Session test that routes `Stop(sink/rec)` through the Kernel; the Sink crate's own-target test covers the guard in isolation. No implementation files have been committed for Steps 9–15.
