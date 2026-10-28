# Brief: hardware probe `hw_tx200_hot_path_probe` (test code only, not risky)

Repository: /home/komatsu/local_work/Ez-SDRv4 (main tree; you are the only editor of it now). Another implementer works in ../Ez-SDRv4-3c3b — do not touch that directory.

## Purpose
On X300 #2, uhd-tx underflows at 200 MS/s on one channel (and at 40 MS/s/channel grouped) while UHD's benchmark_rate holds 200 MS/s. Investigation (read it: ~/.cache/ezsdr-fixes/2026-10-26-stage3/tx200-sol-answer.md) suspects that the throughput tests repeat a waveform of only 1 000 samples (`super::pn(1_000)` in `measure_tx_throughput`, crates/ezsdr-radio-uhd/tests/common/measurement.rs ~3374, and the multi-channel equivalent in tests/common/measurement/multichannel.rs ~627), so every send is ≤1 000 samples per channel (500 grouped), and per-send overhead dominates. The probe tells on the bench whether a longer repeat waveform removes the underflow.

## Change (smallest that works)
1. Make the repeat waveform length of the TX throughput trials a parameter with default 1 000, so the record suite is unchanged: read an env var `EZSDR_TX_WAVEFORM_SAMPLES` (usize, default 1000, panic with a clear message if unparsable) in `measure_tx_throughput` and in the multi-channel TX throughput path. Keep the total-samples semantics each path has today (multichannel splits the artifact over channels — keep that, the env var is the artifact's complex-sample count).
2. Add `#[test] #[ignore] fn hw_tx200_hot_path_probe()` in crates/ezsdr-radio-uhd/tests/hardware.rs, next to the other measurement probes, reusing the existing machinery (look at how `hw_measurements_record` builds a `MeasurementPlan` and calls `measure_tx_throughput` / the multichannel TX throughput; do not re-implement Run construction). It runs, for each waveform length in `[1_000, 2_000, 20_000, 200_000]` (overridable by `EZSDR_TX200_LENGTHS`, comma-separated):
   - single channel 200 MS/s, 30 s (override `EZSDR_TX200_SECONDS`);
   - only if the selector names two TX channels: grouped 40 MS/s/channel and split 100 MS/s/channel, same duration.
   For each trial print ONE line: `TX200 len=<n> shape=<single|grouped|split> rate=<hz> class=<TrialClass> underflows=<count> sends=<blocks if available> samples=<accepted> first_failure=<code,tick,channel if available>`. Use what the group's observation JSON already holds (underflow counts, async_failures, accepted samples/blocks); print the observation JSON compactly after the line if a field is missing rather than adding new plumbing. Use a plan with only the rates needed (not the whole sweep).
   The test must not assert success; it asserts only that each trial ran (it is a diagnostic). Respect existing RF safety constants (SAFE_TX_HZ / bench frequency helpers as the throughput path already does).
3. Nothing else: no production code change, no design/ or KNOWN_LIMITATIONS change, no mutation rows (test-only).

## Rules
- Never run `cargo fmt`/`rustfmt`. Match the surrounding style by hand (long lines are fine).
- Never run mutate.py or remote_mutate.py.
- Build and test: `cargo build -p ezsdr-radio-uhd --features uhd --tests` cannot link without UHD on the host; instead check with `cargo check -p ezsdr-radio-uhd --tests` (the hardware test file is behind the `uhd` feature? — check how `hardware.rs` is gated and use the docker image if needed: `docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target -w /work -e CARGO_HOME=/cargo-target/.cargo-home ezsdr-v4-dev:uhd4.10 cargo test --release -p ezsdr-radio-uhd --features uhd --test hardware --no-run`). Also run `cargo test -p ezsdr-radio-uhd --test fake` to show the default path is unchanged. Test temp files stay on /tmp.
- Do not commit. No self-review round needed.

## Report
Write ~/.cache/ezsdr-fixes/2026-10-26-stage3/report-tx200probe.md: the diff summary (files, functions), the exact bench command to run the probe on X300 #2 (addr=192.168.40.37, selector `{"rx":["0:A:ubx:0","0:B:cbx:0"],"tx":["0:A:ubx:0","0:B:cbx:0"]}` via EZSDR_MEASUREMENT_SELECTOR if the measurement path reads it — check), its expected run time, and the build/test output lines. Your final message is the report's summary.
