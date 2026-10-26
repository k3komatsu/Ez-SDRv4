# 段階 3b-2：報告

## 実装の報告（`report-3b2.md`）

STATUS: done

## Phase 8 stage 3b-2 — Provider measurement groups

HEAD was `3678f2c`. The five Provider-path measurement groups, their record integration, and FakeDevice rehearsal are implemented. No `src/` code, version, schema, or changelog was changed. Nothing was staged or committed.

### Changes

- `crates/ezsdr-radio-uhd/tests/common/measurement.rs`: added plan defaults/validation; Manifest evidence extraction and trial classification; the command-lead, TX in-flight, cold RX-start, RX-throughput, and full-duplex measurement groups; candidate summaries and measured-row construction; and record merging. `build_record` accepts the existing `MeasurementGroup` rather than growing to eight parameters (also keeps workspace clippy clean). Known-tick reports outside trial intervals no longer poison every candidate. `hardware_time` now requires both a successful trial and the requested capture sample. Initial unmeasured reasons distinguish “not run” from candidates tried and failed.
- `crates/ezsdr-radio-uhd/tests/fake.rs`: runs all five same measurement functions with the small rehearsal plan, checks observation shape and counts without asserting fake classifications or measurement magnitudes, writes and reads the record, and verifies FakeDevice rows stay unmeasured with the required reason.
- `crates/ezsdr-radio-uhd/tests/hardware.rs`: validates the plan before opening the device, adds the five groups after the raw probes, and writes the record only after comparison succeeds.
- `design/18-uhd-radio.md`: adds the UR-34 test citations and a Changes row.
- `plan/maintenance/tools/mutations.json`: adds P744–P761; updates P723 for the `MeasurementGroup` record input and makes P746 surgical. P751 now targets the successful-trial-gated `hardware_time` expression.

The only deliberate deferral is the exact UHD `BurstAck` timestamp: evidence is still required at the expected exclusive-end tick. The review’s possible one-sample-earlier timestamp is unverified; allowing it now could accept an incomplete burst. A one-line `shortcut:` comment points to the UHD 4.10 bench observation needed before relaxing this check.

### Test mapping

| Measurement | Evidence and row tests |
|---|---|
| `radio.timing.min_timed_command_lead_ns` | `manifest_evidence_extracts_six_classes_and_rejects_boundary_acknowledgements`; `measured_rows_keep_the_selected_boundary_and_limiting_condition` |
| `radio.timing.tx_in_flight_ns` | `classify_trial_failure_origin_and_completion_evidence`; `incomplete_or_unattributable_trials_never_pass`; manifest evidence and measured-row tests above |
| `radio.timing.rx_start_lead_ns` | `cold_capture_evidence_requires_two_contiguous_clock_segments`; measured-row test above |
| `radio.perf.rx_bytes_per_s` | `throughput_success_requires_sample_tolerance_and_no_overflow`; measured-row test above |
| `radio.full_duplex`, `radio.hardware_time` | `loopback_success_separates_waveform_from_requested_sample_time` |
| Record derivation and environment counts | `measurement_group_merges_only_supplied_comparable_values`; `measurement_plan_from_env_uses_defaults_and_rejects_invalid_counts` |
| Fake rehearsal | `provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements` |

### Self-review and disposition

One read-only review was run with `opencode/deepseek-v4.1-flash-free#max` against `checklist-3b2.md` (P1: 0; P2: 3; P3: 5). No second review was run.

- **P2-1 — fixed:** failed trial classifications now force `hardware_time=false`; the synthetic assertion and mutation P757 cover it.
- **P2-2 — fixed:** known-tick events before/after the trial range are ignored for trial classification; synthetic before/after cases and P756 cover it. Missing-tick evidence remains conservative.
- **P2-3 — deferred, not accepted as a basis to weaken success:** keep requiring the exclusive-end tick until the permitted hardware bench confirms UHD’s `BurstAck.time_spec` semantics. This remains an explicit risk for the two TX timing measurements.
- **P3-1 — fixed:** “not run” and “candidate failed” now have distinct reasons; the record test and P760 cover the distinction.
- **P3-2 — fixed:** restored the existing UR-34 indentation; added citations without unrelated reindentation.
- **P3-3 — fixed:** comparison now precedes the required-path write.
- **P3-4 — fixed:** added record-merge and `from_env` tests; added P758–P761; P746 now removes only the `Underflow` classification arm.
- **P3-5 — recorded:** kept the rehearsal in the default suite because it exercises the requested end-to-end record path. It took about 31.17 s per run; the required 20-run final batch took 623.49 s (about 10m23s).

### Checks

- Focused measurement tests: **13 passed**.
- `cargo test --workspace --no-fail-fast`: **1,789 passed, 0 failed** across 68 harnesses.
- Fake rehearsal, sequential final batch: **20/20 passed**.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Docker UHD-feature clippy (`ezsdr-v4-dev:uhd4.10`, `--features uhd --all-targets -- -D warnings`): passed; no hardware test was run.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: **0 problems**.
- `git diff --check`: clean.
- Selected remote mutations: **41/41 killed** — P715–P719, P722–P723, P725–P732, P735–P742, and P744–P761. No local mutation was run.
- All **1,113** maintenance mutation rows have unique IDs and exactly one `old` match; changed files are mode 644.

### Bench duration estimate

With the default plan, nominal scheduled device time is **about 13.4 minutes**, inferred from the code rather than measured: seven 60-second RX-throughput Runs (420 s), about 350 s for 14 cold-change candidates × 100 alternating-rate captures plus intervals, about 29 s for the two TX sweeps, and about 1.5 s for loopback, plus small settling intervals. This excludes Run/device setup, configuration delays, and host/hardware overhead; actual bench wall time may be longer.

### Not run / remaining risk

No hardware bench was run, as required. The exact UHD `BurstAck` tick remains unverified and must be inspected in a hardware record before changing the strict check. The full mutation lists and Python suites were outside this stage’s gate and were not run.

## 修正の報告（`fix-3b2-report.md`）

STATUS: done

## Phase 8 stage 3b-2 — adjudicated fix report

All six rulings in `fix-3b2-brief.md` are implemented. Repository changes remain unstaged and uncommitted; nothing was committed or pushed.

### Rulings closed

1. **BurstAck association and offset** — `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1362-1398` associates ACKs to the isolated trial at/after its target, accepts one ACK without requiring equality with the exclusive burst end, classifies none as incomplete and multiple as unattributable, and records tick offsets in observations (`:1549-1551`). Synthetic coverage: `manifest_evidence_classifies_tx_ack_and_out_of_range_failure_evidence` (`:1753`). Mutations P762-P764 killed.
2. **Preemption evidence and out-of-range failures** — a preempted repeat must have exactly one EOB record ending at the interrupt target (`:1301-1315`, `:487`). Failures before the first or after the last trial remain attributable to the boundary trial. Tests: `preemption_requires_the_repeat_record_to_reach_the_interrupt_target` (`:1814`) and manifest evidence test above. Mutations P736, P756, P765-P767 killed.
3. **Receive throughput interval** — expected samples come from the recorded SampleClock origin-to-end interval; a missing end is incomplete, and the one-packet boundary is inclusive (`:1113-1121`, `:1155-1167`). The test covers delayed shutdown, missing end, overflow, deficit, and exactly one packet of difference (`throughput_success_requires_sample_tolerance_and_no_overflow`, `:1873`). P748, P749, P768, P769 killed.
4. **Record before profile assertion** — structurally invalid records are refused, while structurally valid records are written before profile-value disagreements are returned (`:289-301`); `hardware.rs:1522-1524` asserts only after the write. `profile_disagreement_is_written_after_structural_validation` (`measurement.rs:2107`) verifies both paths. P770, P771, P774 killed.
5. **Receipt-time lead** — `tx_trial` derives the effective lead and trial start from the submitted entry’s `entry.time` (`measurement.rs:834-838, 879-895`). The pure test uses distinct receipt and later timestamps (`:1629`); the FakeDevice test does not assert measurement magnitudes. P772 killed after correcting its mutation: the original `run.now()` mutant survived because the rehearsal’s manual clock did not advance between the two reads. The replacement mutation shifts the receipt by one tick to model a later read and is killed by the receipt-time test.
6. **Limiting condition and cold transition rates** — numeric rows use `summary.limiting_condition` (`:1585-1589`); cold-stage observations record both rates and every alternating transition (`:1000-1002, 1557-1571`). The existing `Condition` still represents one sample rate per stream, so the pair/direction is carried in the observation. Tests: `measured_rows_keep_the_selected_boundary_and_limiting_condition` (`:1900`) and `cold_observations_record_both_rates_and_each_transition_direction` (`:1640`). P752 and P773 killed.

### Checks

- Focused measurement tests: **17 passed**.
- `cargo test --workspace --no-fail-fast`: **1,793 passed, 0 failed**, across 68 successful harness results; the FakeDevice measurement rehearsal ran.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Docker UHD-feature clippy (`ezsdr-v4-dev:uhd4.10`, `--features uhd --all-targets -- -D warnings`): passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: **0 problems**.
- `git diff --check`: clean. All 1,126 maintenance mutation rows have unique IDs and exactly one `old` match; changed source/ledger files are mode 644.
- Selected remote maintenance mutations: initial run killed 20/22; after the fixes, P748 and P772 both killed (2/2). Across the selected set, all 22 rows are killed. No local mutation was run.

### Not run / remaining

- No hardware bench was run. The FakeDevice rehearsal was not repeated 20 times in this repair round because its test was unchanged; it ran in the focused/full suites, and the earlier 20/20 batch was before this repair.
- No self-review or Sol re-review was run in this session. The full mutation lists and Python suites were not run.
- The exact UHD `BurstAck` timestamp semantics remain unverified; the observation now preserves ACK offsets for the hardware record rather than assuming a tolerance.
