STATUS: done

# Phase 8 — Stage 3c-3b report

## Delivered

- `crates/ezsdr-radio-uhd/tests/common/measurement.rs`: guarded reopen eligibility, conservative merge of reopened RX-throughput/start-lead measurements, invalidation when an applicable reopened result is skipped or fails, and a reopened RX sweep limited to the single-channel conditions.
- `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs` (new): H4 Session-to-Provider timed RX-gain probe, signed dispatch-delay mapping, per-channel TX submission-headroom proxy, clock/thread/CPU observations, and pure calculation tests.
- `crates/ezsdr-radio-uhd/tests/common/probe_device.rs`: bracketed `time_now` and timed-apply entry observations, plus TX-send streamer generations.
- `crates/ezsdr-radio-uhd/tests/fake.rs`: reopen lifecycle and conservative-value tests; H4 output-wiring, not-submitted-row, error/EOB, and shell checks. The H4 rehearsal selects slot-B CBX to demonstrate it is not limited by raw-probe slot-A applicability.
- `crates/ezsdr-radio-uhd/tests/hardware.rs`: `hw_h4_dispatch` is a separate record probe, so the raw-probe slot-A filter does not suppress it.
- `design/18-uhd-radio.md`: UR-34 wording and test names updated for reopen and H4 behavior.
- `plan/phase8/bench.md`: BP2 load accounting and BP4c reopen rules and estimates.
- `plan/phase8/tools/h4-load.sh` (new, mode 755): bounded PID-managed CPU/memory workers, shutdown/reap traps, and `--self-test`.
- `plan/phase8/tools/run-record.sh`: H4 load lifecycle, readiness/early-exit recording, help text, and PID cleanup on success, error, or signal.
- `plan/maintenance/tools/mutations.json`: P1124–P1147, with the P1138/P1139 indentation corrected.

No `src/` files, production values/defaults, or schemas were changed. No hardware was run.

## Decisions and estimates

- Reopened RX throughput repeats the single-channel sweep only; multi-channel reopen throughput was intentionally omitted. Successful throughput uses the lower base/reopened value; start lead uses the higher value.
- A skipped applicable reopen invalidates the earlier values, as required. The review suggestion to retain base-only values on skip was rejected because it conflicts with that requirement.
- H4 uses a test-only `ezsdr.h4.set_rx_gain` Vocabulary verb to exercise timed gain through Session → Provider. The observations are proxies, not exact scheduler wake-up latency or RF execution time.
- `bench.md` estimates the no-load record at about 60–75 minutes for #1/OBX and 1 h 35 min–1 h 55 min for the two-channel #2 setup; these totals include about 9–10 minutes for reopen (INFERRED). H4 load covers the whole record and can extend those times; no fixed loaded estimate is claimed. Set `EZSDR_MEASUREMENT_H4_DURATION` longer than the complete run. H4 itself has about a 2-second update window plus Run setup/stop.

## Verification

- Focused reopen lifecycle and reopened-value tests passed.
- H4 fake tests passed; the timing-sensitive `h4_fake_session_records_timed_rx_gain_and_tx_submission_proxies` rehearsal passed 20/20 consecutive runs.
- `cargo test --workspace --no-fail-fast`: passed, 2,034 tests across 68 targets; no failures or ignored tests.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- UHD-enabled Docker clippy using `ezsdr-v4-3c3b-target`: passed.
- `h4-load.sh --self-test`: passed; workers stopped and were reaped. `bash -n` passed for both scripts. The fake test also checks `run-record.sh --help` contains all H4 variables.
- `rules.py --base HEAD`: 0 problems. `rules.py --self-test`: passed. `git diff --check`: passed.
- Targeted remote mutation run on sim02/sim03, IDs P1124–P1147: 24/24 killed.

## Self-review

One self-review was completed using the authorized Go fallback after the free-model limit. It reported 1 P1, 3 P2, and 8 P3 findings. No second review was run.

| Finding | Severity | Handling |
|---|---|---|
| 1. H4 fake rehearsal did not assert computed dispatch/headroom results | P1 | Fixed: assert mapped dispatch interval, apply-entry timestamp, host-clock brackets, and at least one computed headroom interval/proxy. |
| 2. Skipped reopen retaining base values | P2 | Rejected: skipped applicable reopened measurements must invalidate earlier values. |
| 3. TX headroom EOB/error boundaries lacked test coverage | P2 | Fixed: added EOB and error-boundary cases and mutation rows. |
| 4. H4 incorrectly shared raw slot-A applicability | P2 | Fixed: run H4 separately; fake rehearsal uses slot-B CBX. |
| 5. Startup-failure path waited twice and misreported status | P3 | Fixed: route early helper exit through one cleanup/wait path and record startup parameters/PID. |
| 6. `not_submitted` rows lacked setting identity | P3 | Fixed: rows include direction, channel, setting, and instant; tested and mutated. |
| 7. No Bash syntax test | P3 | Fixed: fake suite runs `bash -n` on both scripts. |
| 8. Restore-failure/unwind reopen refusal untested | P3 | Fixed: added both sequences and verified the opener is not called. |
| 9. Base-unmeasured/nonnumeric merge cases untested | P3 | Fixed: added both cases and mutation rows. |
| 10. Mutation indentation and `h4.rs` mode | P3 | Fixed: rows aligned; `h4.rs` mode is 644. |
| 11. Memory-buffer accounting unclear | P3 | Fixed: BP2 distinguishes source/target buffers and the three-buffer budget. |
| 12. H4 variables absent from `run-record.sh --help` | P3 | Fixed: help lists CPU, memory, and duration variables; tested. |

## Not done

No hardware measurement, Python suite, local mutation run, `cargo fmt`, staging, commit, or push was performed. Work remains uncommitted on `wip/3c3b`.

## Fix round 1

1. **TX underflow clock conversion (P1):** Convert report timestamps through the Manifest's SampleClock records; count unconvertible reports and mark the TX interval ambiguous. The fake H4 test injects an underflow after the first accepted TX send and asserts the event came from the TX SampleClock, was converted, and was counted. Mutations P1148–P1149 and P1155–P1157 killed. P1148 initially survived with a wall-time fault, so the fixture was changed to inject at the active TX send boundary; it then killed.
2. **Early TX end (P1):** Mark the H4 observation partial when TX ends early or its concurrent interval is ambiguous, and exclude gain applies after TX end from the concurrent delay distribution. The same fake integration test keeps all four gain applies successful while TX underflows. Mutations P1150–P1151 killed.
3. **Bounded probe rows (P2):** Retain at most 65,536 rows, count drops, and look up only the matching timed apply while holding the probe lock. Row-cap and H4 integration tests pass. Mutations P1153–P1154 killed.
4. **Forward clock jumps (P2):** Reject host/device anchor spans outside 1,000 ppm plus measured bracket uncertainty. Synthetic monotonic-jump test passes. Mutation P1152 killed.
5. **Signal cleanup (P2):** Run the measurement container in the background and wait on its PID; cleanup stops it by name and stops/reaps H4 load. Manual dummy-command check: substituted a long `sleep` for the Docker measurement, sent TERM to the runner, and confirmed the runner, measurement process, and H4 worker stopped. No mutation row covers this script lifecycle check.

**Verification:** focused H4 tests 11/11; the timing-dependent underflow integration test 20/20; full fake suite 363/363; workspace and UHD-enabled Docker clippy passed; `rules.py --base HEAD` reported 0 problems and `--self-test` passed; `git diff --check` and `bash -n` passed. Targeted remote mutations P1148–P1157 all killed (10/10 across the initial and fix-round reruns). No hardware was run. Changes remain unstaged and uncommitted on `wip/3c3b`.
