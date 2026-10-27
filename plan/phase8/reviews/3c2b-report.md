# 段階 3c-2b：報告

## 実装の報告（`report-3c2b.md`）

STATUS: done

## Stage 3c-2b — implementation report

### Delivered

- `crates/ezsdr-radio-uhd/tests/hardware.rs`: six ignored, observation-only probes: H1, H2, H5, both #56 cases, and UR-17. `hw_measurements_record` calls all six and its expected-name set includes exactly those six new observations.
- `tests/common/observations.rs`: heard intervals with loopback-delay correction, PN correlation with weak/tied-peak reporting, and report-code counts; four synthetic tests.
- `tests/common/probe_device.rs` and `tests/common/mod.rs`: the test-only forwarding `Device` for host-timed RX call recording, pausing the Provider reader, and draining the device without a concurrent reader.
- `design/18-uhd-radio.md`: UR-34 names the six probes and four pure tests; one Changes row cites design note 41, stage 3c-2b.
- `plan/maintenance/tools/mutations.json`: P848–P851 cover heard-interval delay correction, PN offset, tied-peak ambiguity, and missing report codes.

No production `src/` change, measured value row, `unmeasured_reason` change, fake rehearsal/fault, version change, or schema changelog entry. H3 and H4 remain deferred.

### Probe/test names

| Probe | Test name | Trials/inputs |
|---|---|---|
| H1 | `hw_h1_late_multi_packet_burst` | 18: three repeats at each of 1/5/10 ms late, in all-timed and timed-head/untimed-tail modes. Records every send, async report, and heard interval. |
| H2 | `hw_h2_underflow_mid_burst` | 3: timed 10,000-sample head, 10 ms starvation after its planned end, untimed 10,000-sample tail; partitions observed and unobserved Underflow and correlates PN prefixes. |
| H5 | `hw_h5_held_last_sample` | 200 adjacent-control and 200 positive-gap (3 ms) Session bursts; records each case’s actual burst records, submits, reports, events, and termination. The QPSK components are scaled by 0.7, giving complex magnitude below 0.5. |
| #56 Stop between starts | `hw_56_stop_between_starts` | 3: cold RX rate change, Session Stop, device-level drain, and recovery capture; records nominal and host-observed device-call ordering separately. |
| #56 Stop lag | `hw_56_stop_lag` | 20 Stops; compares submit brackets with the forwarded `rx_stop` call on one host monotonic clock and retains Session log times and Provider shutdown timing. |
| UR-17 | `hw_ur17_receive_call_duration` | 1,600 raw receives across 390,625/1/10/50 MS/s and 2,000/65,536 samples, plus a paused-reader cold-cut observation. |
| Synthetic helpers | `heard_intervals_cover_silence_midstream_and_a_truncated_tail`; `correlation_offset_finds_zero_positive_negative_and_noisy_offsets`; `correlation_offset_marks_equal_peaks_ambiguous`; `report_counts_include_all_codes_and_malformed_rows` | Four pure tests; all passed in the final workspace test run. |

#### Changes from the brief and reasons

- Followed §7 where it supersedes the table: H1 includes both send modes; H2 uses PN correlation and a host-clock starvation deadline; H5 includes an adjacent control; #56 records device call order and drains at device level before recovery; UR-17 includes 390,625 S/s and an overrun-near-cut observation.
- UR-17’s timed receive start is now scheduled 10 ms ahead and rounded to a sample boundary at every rate; a fixed sample-count lead would be too short at high rates.
- H5 waits 2 s after each case so its 200 requested bursts can complete, and records per-case burst records plus the full section and `termination.undelivered` evidence.
- A temporary direct `serde` test dependency was removed after the workspace governance test rejected it. Final observation JSON serializes only the required fields; no manifest dependency change remains.

### Bench time estimate

Not measured. Approximate quiet-bench time is about 65 seconds: UR-17’s requested sample blocks alone represent about 49.7 seconds at the four rates; H5 advances about 4 seconds; H1/H2 and the three #56 ordering trials account for several more seconds. The 20 #56 lag trials each allow a 1-second poll for `rx_stop`, so a slow/non-arriving call can add up to 20 seconds; a practical upper estimate is about 85 seconds, excluding unusual device/control delays.

### Mutations and checks

- Remote selected mutations P848, P849, P850, P851: final result **4/4 killed**. P850’s first mutant (`>` instead of `>=`) was too weak because an equal peak still exceeded 80% of the best peak; it survived. The mutant was strengthened to a 110% ambiguity threshold, then the selected P850 rerun was killed. The tests and production logic were unchanged for this correction.
- The initial remote runner attempt stopped before executing mutants because its scratch copy could not read a prior-review artifact. I left that artifact’s permissions alone and ran the selected rows from a temporary source copy excluding review/local-tool state.
- `cargo test --workspace`: pass after removing the temporary dependency; this run included all four pure helper tests. An earlier run correctly caught the temporary dependency’s PO-8 violation.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- UHD-feature compile/lint after the self-review fixes: Docker `cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`: pass. No ignored hardware test was run.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problems. `git diff --check`: clean.

### Self-review

One read-only self-review, model `opencode-go/deepseek-v4.1-flash#max`; no Sol review was run. All findings were fixed:

- **P1:** H5’s original unscaled PN had magnitude about 0.566 and failed the 0.5 RF assertion. Scaled both components by 0.7 and retained the pre-send assertion.
- **P2:** H5’s 1.5-second advance could leave late positive-gap bursts undelivered. Increased each advance to 2 seconds and added per-case burst records and termination evidence.
- **P2:** UR-17’s 10,000-sample start lead fell to 0.2 ms at 50 MS/s. Replaced it with a 10 ms device-time lead rounded to the sample grid.
- **P3:** Added H5 rate/range and case-specific burst records; added explicit #56 event counts; classified #56 ordering from the forwarded host-time RX call intervals and renamed the log-time result `stop_placement_nominal`; moved imports to the surrounding convention.

### Remaining

Hardware behavior remains unobserved: all six probes are still `#[ignore]` and require a USRP. No values are claimed, and no hardware timing, RF behavior, or cleanup result has been measured. Changes remain uncommitted and unstaged.

## 修正の報告（`fix-3c2b-report.md`）

STATUS: done

## Fix 3c-2b report

### Adjudicated findings

- **Sol 1 — #56 RX restart:** Both probes now issue `start_rx` to `radio/rx/s0`. The recovery capture is submitted only after the forwarding Device records a successful `rx_start` and a later sample receive. Stop-lag results count only attempts with a recorded `rx_stop` and successful restart/sample evidence; other attempts are stored separately, and a failed recovery ends the sequence before another Stop. The stop-lag record also retains the RX device calls for diagnosis. (`crates/ezsdr-radio-uhd/tests/hardware.rs:1732-1745, 1876-1959, 1968-2020`; evidence predicates at `crates/ezsdr-radio-uhd/tests/common/probe_device.rs:33-38`.)
- **Sol 2 — H5 deadline path:** The adjacent and prebooked positive-gap controls remain. A third positive-gap case submits one burst at a time and submits its successor only after the forwarding Device records a successful EOB. TX records include host-time brackets, per-channel sample counts, requested time, SOB/EOB, timeout and result. The observation still explicitly says it is not a deadline guarantee. (`crates/ezsdr-radio-uhd/tests/hardware.rs:1747-1867`; TX recording and EOB predicate at `crates/ezsdr-radio-uhd/tests/common/probe_device.rs:28-31, 176-192`.)
- **O1 — drain recovery:** `drain_without_reader` now resumes the Provider reader through a drop guard, including during unwinding. A synthetic panic test pins this behavior. (`crates/ezsdr-radio-uhd/tests/common/probe_device.rs:22-25, 78-99, 230-244`.)
- UR-34 names the three new evidence tests and records the corrected H5/#56 behavior. Mutation rows P852–P858 cover the EOB, restart-evidence and unwind decisions. (`design/18-uhd-radio.md:361, 697`; `plan/maintenance/tools/mutations.json:11945-12014`.)

### Checks

- `cargo test --workspace`: passed. The three new synthetic tests passed. An initial run exposed that the panic fixture started the fake RX stream in the past, so its late-command path bypassed the panic; the fixture now schedules its start in the future, and the final run passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Docker UHD-feature clippy (`cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`): passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problems.
- `git diff --check`: clean.
- Remote selected mutations, run from the working tree with `remote_mutate.py maintenance sim02:4,sim03:4 P852 P853 P854 P855 P856 P857 P858`: **7/7 killed**. P848–P851 were unchanged by this fix.

### Bench estimate and remaining

The revised quiet-bench estimate is **about 68 seconds for successful trials, up to roughly 90 seconds if stop-call polls or a recovery wait approach their timeouts** (INFERRED, not measured). The serial H5 case adds about 3.4 seconds for 200 × 17 ms advances; the previously ineffective #56 restarts no longer cause repeated one-second missing-stop waits when the device responds normally.

The six hardware probes remain ignored and require the USRP bench; no hardware behavior or cleanup result was observed. No Sol review or self-review was launched, as instructed. Changes remain unstaged and uncommitted.

## 修正 2 の報告（`fix2-3c2b-report.md`）

STATUS: done

## Fix 2 — Stage 3c-2b report

### Adjudicated findings

- **Sol 1 — incomplete H5 EOB:** `successful_tx_eob` now accepts an EOB only when every recorded channel sample count equals the accepted count; a one-sample EOB returning zero is rejected by the synthetic test (`crates/ezsdr-radio-uhd/tests/common/probe_device.rs:30-35,216-223`). H5 polls an atomic EOB-attempt counter rather than repeatedly cloning the full log, records `accepted` / `incomplete` / `not_observed`, preserves incomplete EOB rows separately, and stops the serial sequence unless the full EOB was accepted (`probe_device.rs:19,69-71,115-119,227-235`; `crates/ezsdr-radio-uhd/tests/hardware.rs:1751-1762,1813-1850`).
- **Sol 4 — mutation descriptions and coverage:** P856/P857 now name the equality cases their `>=`→`>` mutants reject. P859/P860 remove each ordering predicate to verify that stale host-time and device-tick samples are rejected (`plan/maintenance/tools/mutations.json:11985-12032`).
- **Sol 5 — measurement caveat:** H5’s observation explicitly says it comes from an instrumented idle bench, that recorder overhead was not measured, and that it is not a deadline measurement (`hardware.rs:1878`).

### Checks

- `cargo test --workspace`: passed. The synthetic tests `successful_tx_eob_requires_an_accepted_eob`, `tx_eob_count_tracks_recorded_eob_sends`, and `receive_sample_follows_a_successful_device_start` passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Docker UHD-feature clippy (`cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`): passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problems, including exact-once checks for mutation source rows across the lists.
- `git diff --check`: clean.
- Remote selected mutations P852–P861 on sim02/sim03: **10/10 killed**.

### Remaining

No hardware probe was run; H5 hardware behavior and recorder overhead remain unmeasured. No Sol review or self-review was run, as instructed. Changes remain unstaged and uncommitted.
