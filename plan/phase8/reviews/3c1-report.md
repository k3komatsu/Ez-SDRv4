# 段階 3c-1：報告

## 実装の報告（`report-3c1.md`）

STATUS: done

## Phase 8 stage 3c-1 report

Implemented the TX start-lead, RX overflow restart-gap, and TX throughput measurement groups for `hw_measurements_record`, with FakeDevice rehearsal coverage. No `src/` files or hardware were changed/run; the changes remain unstaged and uncommitted.

### Files changed

- `crates/ezsdr-radio-uhd/tests/common/measurement.rs`: measurement plans, Provider-path measurements, evidence classification, observations, and pure tests.
- `crates/ezsdr-radio-uhd/tests/fake.rs`: wires all three groups into the round-trip rehearsal and checks their observations and unmeasured FakeDevice values.
- `crates/ezsdr-radio-uhd/tests/hardware.rs`: wires the three groups into `hw_measurements_record`.
- `crates/ezsdr-radio-uhd/tests/README.md`: describes the rehearsal’s scope.
- `design/18-uhd-radio.md`: updates UR-34, its test references, and Changes.
- `plan/maintenance/tools/mutations.json`: adds rows P776–P790.

### Measurement decisions and tests

- **`radio.timing.tx_start_lead_ns`**: sweeps the seven configured leads over both rate pairs, including `390625 ⇄ 1000000` S/s. Success requires `tx_switch.at < origin` and no classified failure evidence. The burst-at-origin criterion is omitted because the test thread can reserve it too late; this is recorded in the observation. An admission rejection is recorded as `AdmissionRejection`, not allowed to abort the whole record. Tests: `tx_start_lead_requires_an_early_switch_and_no_failure_evidence`, `tx_start_lead_value_uses_the_successful_suffix_intersection`.
- **`radio.timing.overflow_restart_gap_ns`**: requires one overrun, one matching `overflow_restart` continuity gap, `lost == len`, matching payload loss, a complete capture, and the requested SampleClock rate. The reported value is the maximum complete gap. Tests: `overflow_evidence_requires_one_overrun_one_matching_gap_and_the_measured_rate`, `overflow_wait_horizon_allows_capture_to_finish_after_the_stall`, `overflow_summary_uses_the_maximum_and_refuses_an_under_trialed_rate`.
- **`radio.perf.tx_bytes_per_s`**: uses the recorded TX SampleClock interval, rejects underflow and non-`BurstAck` async reports, and allows one RX `max_num_samps` as packet tolerance. `Device` has no TX packet-size query, so that tolerance is explicitly inferred from RX. Test: `tx_throughput_uses_the_ended_clock_interval_and_rejects_missing_samples_or_underflow`.
- The integrated FakeDevice check is `provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements`; the hardware entry point is `hw_measurements_record`.

The overflow observation and this report define the value as the **full RM-17 continuity gap**, including loss during the configured reader stall and socket-backlog drain—not UHD recovery time alone. A restart-only measurement would need reader-resumption evidence, which this scoped test path does not expose.

### Default hardware-time estimate for these three groups

Estimate only; no hardware run was performed. This is requested device horizon, excluding per-Run startup/cleanup and host/network overhead. The short packet-size lookup Session is included in the count but has no added horizon.

| Group | Runs/Sessions | Requested device horizon |
|---|---:|---:|
| TX start lead: 7 leads × 2 conditions, 100 transitions each | 14 Runs | ~66.6 s (candidate lead plus 20 ms inter-trial interval) |
| Overflow: 4 rates × 20 trials | 80 Runs | 1,180 s (19 min 40 s) |
| TX throughput: 7 rates × 60 s | 7 Runs | 420 s (7 min) |
| RX packet-size lookup for TX tolerance | 1 Session | Not separately advanced |
| **Total** | **102 Sessions** | **~1,666.6 s (27 min 47 s)** |

The overflow horizon is `2000 ms + after_ms + 2 × stall_ms + 500 ms`; default stalls are 20 s, 2 s, 1 s, and 0.5 s at 1, 10, 25, and 50 MS/s. Relative to the prior one-stall allowance, the complete-capture requirement adds 7 min 50 s to these default overflow horizons. The 1 MS/s trial’s 20 s configured stall can dominate the recorded maximum; that value must not be read as a standalone measurement of UHD’s restart latency.

### Self-review and dispositions

One read-only review completed with OpenCode Go `deepseek-v4.1-flash` after the free model hit its usage limit; no second review was run. It reported 1 P1, 4 P2, and 4 P3 findings.

- **P1 — overflow capture horizon was too short:** fixed by adding one stall-length drain allowance; added a complete-capture test and P789. The expectation is now 43,000 ms for a 500 ms pre-stall interval and 20 s stall.
- **P2 — partial captures lacked a negative test:** added `partial: true` → `Incomplete` coverage and P788.
- **P2 — normal repeat `BurstAck` lacked coverage:** added a success assertion and P787.
- **P2 — TX cold-rate admission rejection could abort the measurement:** now carries admission into trial classification and has a pure rejection assertion; added P790.
- **P2 — overflow value scope was unclear:** observation and report now state that the gap includes stall/backlog loss and is not UHD recovery time alone.
- **P3 — duplicate test reference in UR-34:** removed the duplicate; the test is listed once in the provider-measurement test group.
- **P3 — FakeDevice rehearsal grew from 31.16 s to 43.61 s:** accepted and measured rather than removing the RX packet-size lookup. The device contract returns zero without an open RX streamer (`src/device.rs::ur_33_a_packet_size_read_of_a_stream_with_no_streamer_is_zero`; the UHD test `uhd_streams_are_looked_up_by_number` pins the same behavior), so dropping that Session would invalidate the packet tolerance. The final 20-run rehearsal took 872.3 s total; this is a known test-runtime cost.
- **P3 — initial TX rate was assumed to be 1 MS/s:** retained the X3xx bench-profile precondition, explicitly checks the initial SampleClock rate, and records `initial_tx_rate_hz` in the observation.
- **P3 — count-override docstring said “three”:** corrected to “four”.

### Verification

- Focused measurement tests: **23 passed**.
- Fake rehearsal: **20/20 passed**, 43.60–43.63 s each (43.61 s mean).
- Workspace tests: **1,799 passed across 68 targets**.
- Workspace clippy and UHD-feature Docker clippy: passed with `-D warnings`.
- `rules.py --base HEAD`: **0 problems**; `git diff --check`: clean.
- Mutation source check: all **1,142 rows** match their `old` text exactly once across 56 source files.
- Remote mutations: **P776–P790 all killed**. P776–P785 initially passed 10/10; after review changes, P776/P786 passed 2/2, the affected/new rows P776, P779, P782–P789 passed 10/10, and P790 passed 1/1. The complete mutation lists were not run.

### Remaining

The hardware measurement has not been run. On the configured X310, run `hw_measurements_record` with the required metadata to collect device evidence and confirm whether each rate completes; the fake rehearsal does not establish measured values. The Python suites were not run because this change does not touch Python and they are outside the stage brief.

## 修正の報告（`fix-3c1-report.md`）

STATUS: done

## Phase 8 stage 3c-1 fix report

Applied the adjudicated TX-throughput and overflow-measurement changes. All edits remain unstaged and uncommitted; no `src/` files or hardware were changed/run.

### Findings and fixes

- **Sol 1 + Sol 2 — TX throughput evidence and interval.** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1719-1768` now anchors the interval to the Run's recorded repeat-burst target, classifies exactly one `BurstAck` at or after start + D as completion, and requires `tx_samples >= D × rate`. Underflow, relevant async failures and short-send rejections remain device failures; zero ACKs are incomplete and multiple ACKs are unattributable. The Run advances from its scheduled burst target through D seconds (`:1795-1803`), and observations include start, ACK, elapsed samples, actual samples and their difference (`:1817-1829`). The RX packet-size lookup and upper sample-count tolerance were removed. Synthetic coverage is in `:2709-2758`.
- **Sol 3 / O1 — overflow gap depends on reader stall.** The full continuity gap and inferred restart-after-resume estimate remain observations; the group supplies no comparable value row (`:1522-1578`). The estimate uses the capture sample clock and `after + stall`, rounds sample conversions up, and makes negative estimates null (`:1320-1360`). Per-rate and cross-rate summaries are observations only (`:1461-1495`). The default trial count is five (`:658-662`). Synthetic checks are in `:2655-2706`; FakeDevice asserts no overflow value is emitted (`crates/ezsdr-radio-uhd/tests/fake.rs:670-719`).
- **TX-switch evidence.** The observation now says `tx_switch.at` marks configuration completion/clock adoption, not transmission (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1180`).
- **Wiring and documentation.** The hardware entry point includes all three groups (`crates/ezsdr-radio-uhd/tests/hardware.rs:1503-1511`); the rehearsal checks classifications, observations and record round-trip (`crates/ezsdr-radio-uhd/tests/fake.rs:631-719`); README scope is updated (`crates/ezsdr-radio-uhd/tests/README.md:27-33`); UR-34 test names and the Changes entry are updated (`design/18-uhd-radio.md:351-359,693`).

### Sol re-review and follow-up

Sol 6.1 re-reviewed the fixes and closed the three original findings. It raised one medium coverage gap: the early-ACK case was before the burst start, not between burst start and the D-second boundary. Added the `tick=3099` assertion against the `tick=3100` boundary (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:2737-2738`) and mutation P806 (`plan/maintenance/tools/mutations.json:11485`). The focused test passed and P806 was killed. No second Sol review was run after this test-only follow-up; the repo workflow requires Sol re-review after a major fix, and this finding changed no implementation behavior. The review log is `sol-fix-3c1-review.log`.

### Default hardware-time estimate

Requested device horizon only; excludes per-Run startup/cleanup and host/network overhead.

| Group | Runs/Sessions | Horizon |
|---|---:|---:|
| TX start lead: 7 leads × 2 conditions, 100 transitions each | 14 | ~66.6 s |
| Overflow: 4 rates × 5 trials | 20 | 295 s (4 min 55 s) |
| TX throughput: 7 rates × 60 s | 7 | 420 s (7 min) |
| **Total** | **41** | **~781.6 s (13 min 1.6 s)** |

Overflow horizons use `2000 ms + after_ms + 2 × stall_ms + 500 ms`; the second stall allowance is for reader drain. Default stalls are 20 s, 2 s, 1 s and 0.5 s. The RX packet-size lookup Session was removed.

### Verification

- Focused measurement tests: 24/24 passed before the added intermediate-ACK assertion; that modified test passed 1/1 afterward.
- FakeDevice rehearsal: 20/20 passed, 45–46 s each. This loop preceded only the later pure synthetic assertion; rehearsal code and production behavior were unchanged by that assertion.
- Final-tree `cargo test --workspace --no-fail-fast`: 1,800 passed across 68 targets; zero failures.
- Final-tree `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Final-tree Docker UHD-feature Clippy (`ezsdr-radio-uhd --features uhd --all-targets`, `-D warnings`): passed.
- `rules.py --base HEAD`: 0 problems. `git diff --check`: clean.
- Mutation source check: all 1,155 rows have unique IDs and each `old` string matches exactly once.
- Remote mutation results: P776–P780, P782, P784–P786, P788–P805 killed (27/27); P806 killed (1/1). Only targeted rows were run, as required; full mutation lists were not run.

### Remaining

No hardware measurement was run. The availability/timing of hardware `BurstAck` timestamps and the overflow restart-after-resume estimate still need bench evidence; the estimate remains explicitly inferred, and `radio.timing.overflow_restart_gap_ns` remains unmeasured. Python suites were not run (Python was untouched). No self-review was run, as instructed.
