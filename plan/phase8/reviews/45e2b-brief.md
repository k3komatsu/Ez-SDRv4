# Brief: note-45 experiment E2b — the speed measurement against today's owner

You are the implementer (AGENTS.md §9). Repository `~/local_work/Ez-SDRv4`, branch `main`. Edit in place: you are the only editor of the tree. Read AGENTS.md first. Do not cd into `v3/`. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`** and do not search for it.

This step is measurement tooling: there is no test plan (owner 2026-10-26), and Sol reviews the code. The self-review is waived.

## Context (read these)
- **Note 45** `plan/phase8/45-uhd-decision-core.md`: §4.5 (the cell's work), §4.8 (speed), and §7's "速さ" criterion. That criterion is binding, and this brief does not loosen it.
- **Plan** `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md`: the E2b row in §2, and §10's E2 rows.
- **Facts** collected for this step, with `path:line`: `~/.cache/ezsdr-fixes/2026-11-01-n45/e2b-speed-facts.md`. Read all of it. It covers today's release path and its locks, the receive block path, the tooling you can reuse (and why `ProbeDevice` and H4 cannot be used as they are), and FakeDevice's pacing.
- **E2a** (`crates/ezsdr-radio-uhd/src/brain/`):
  - `threads.rs`: the runtime of brain, S and `R_k`, with the clock/timer thread K;
  - `shared.rs`;
  - `model_device.rs`;
  - `thread_tests.rs`: smoke and free-run patterns;
  - `test-plan-e2a.md` §16 and §17: results (E2a is committed: e144da8; #76 in a9e8960 added `provider::test_support::released` and pure H4 functions in `tests/common/measurement/h4.rs`).

## Definitions (fixed; do not change them)
- **Delay of one update** = host time of the `Device::apply(.., Some(e))` entry, minus the host instant of `e − release_window`. The release window is 3 ms (`ezsdr-radio/src/x3xx.rs:385`). This is H4's definition (`tests/common/measurement/h4.rs:135`), and it is the same for both owners, so each owner's own release lateness is included:
  - today's 1 ms poll in uhd-control;
  - the new runtime's timer, which delivers the release to the brain at `e − window`.

  On a Real-clock FakeDevice with `drift_ppm = 0`, device ticks map to host time exactly (facts §4). Also record each owner's own release stamp where one exists. They are diagnostics, not the verdict.
- **Applied** = the FakeDevice DSP log (`dsp_log`) shows the value taken at tick `e`. An update that is refused, cancelled, not applied, or applied at a tick other than `e` fails the speed verdict. **Every update in the table is counted.**
- **Block loss** = overflow results plus the gap test's lost samples, per run, for each owner. Use the owner's stats (`rx_overflows`, the gap count) and the new runtime's `R` counters. The new runtime must lose no more than today's owner.
- **Verdict per condition:**
  - p99(new) − p99(today) < 100 µs;
  - p99(new) ≤ 300 µs;
  - loss(new) ≤ loss(today);
  - every update accounted.

  Report p50, p99, p99.9, the number of updates behind each, and the observation time.

## The update table (fixed before any run; write it into the report)
- One receive stream, one channel. Timed DDC frequency updates (`radio.rx.c0.frequency_hz` path, the chain/`hand` path, not gain), alternating two frequencies.
- **Spaced:** an update every 5 ms (longer than the window).
- **Bursty:** groups of 4 updates 100 µs apart, one group every 10 ms. That stays below the 16-command queue and the 32-entry DSP FIFO.
- At least 10,000 updates per condition and owner, so p99.9 rests on at least 10 samples beyond it. The first instant is at least 50 ms after the stream's start.

## Rates (decide by a trial first; report the trial)
1. Try 200 MS/s with `block_len = 2000` (100,000 blocks/s, 10 GbE link selector, facts §2) on a Real-clock FakeDevice, today's owner, for 2 s. If FakeDevice keeps up (no fake `Overflow`, no gap), that is the "200 MS/s" condition.
2. If it does not keep up, run "the 200 MS/s block rate" as 100,000 blocks/s with a smaller block at a lower sample rate (note 45 §7 says "200 MS/s 相当の block の率"). Choose the largest `block_len` that keeps up, and state it.
3. 1 MS/s with `block_len = 2000` (500 blocks/s).

Run every condition for both owners.

## What to build
1. **A light timing decorator** over `Arc<dyn Device>`, test-only, in `src/` so that both owners can use it. It records only:
   - the apply entry host ns with `e`, and its return;
   - per-stream block and overflow counts from `rx_recv` results.

   It keeps no per-block rows and allocates nothing per block (unlike `ProbeDevice`). Preallocate or bound it.
2. **Today's owner harness.** uhd-control and uhd-rx on that decorator over a Real-clock FakeDevice. Updates go in with no Kernel: `tests/fake.rs::Direct` shows how, or use `src/provider/test_support.rs`. If you must move a helper so both harnesses can reach it, say so. No production behaviour changes, and no release hook in production code. Use the proxy definition.
3. **New-runtime harness.** E2a's runtime on the same decorator and FakeDevice configuration. The table's updates are released to the brain by K's timer at `e − window`, as a value release input, the way today's owner releases at its window. `R_k` publishes into the test sink.
4. **Cell instrumentation for the new runtime**, test-only:
   - per-block cell hold time and wait time, with their distributions;
   - publish retries (`unchanged_since` false);
   - settings-take retries.

   For today's owner, record what is cheap: the number of chain-lock acquisitions per block, from the code (CALC), unless a counter exists.
5. **The measurement runs**, as ignored tests (`#[ignore]`, release builds). Each prints one JSON result per condition and owner:
   - delays: p50, p99, p99.9, max and count;
   - every update's outcome;
   - loss;
   - cell metrics;
   - observation time;
   - load average before and after.

   Interleave the owners (A, B, A, B; at least 2 repetitions each) so that machine load drifts equally.
6. **One fault run per owner**, separate from the verdict. A device loss in the middle of the table: report what happened to the remaining updates and to the threads' exits.

Default (non-ignored) tests: only quick self-checks of the decorator and the percentile and accounting code (a few ms each).

## Where to run
The local PC is loaded by the owner's other jobs: its load average was about 22 earlier today. **Run the measurement runs on sim03** (or sim02 if sim03 is busy, checked with `uptime`) through `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cargo command>`, one at a time. Building and the quick tests may run locally. Record the host, its core count and `uptime` in the results.

## Rows
Mutation rows from E2-62 on (E2-60 and E2-61 are taken), only for decisions in the measurement code that change the verdict:
- the percentile index;
- counting an unapplied update as a failure;
- the loss comparison;
- the application-tick check.

Append them to `plan/maintenance/tools/mutations.json` without re-serializing. Then run `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `python3 plan/maintenance/tools/rules.py --base HEAD` (0 problems).

## Out of scope
Production behaviour changes, FakeDevice behaviour changes, `design/`, the Kernel, schemas, Python, and E2a's sequences. If you think the definitions or the table are wrong, say so in the report; do not change them.

Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout. Run `cargo fmt --all`, and `cargo clippy -p ezsdr-radio-uhd --all-targets -- -D warnings` clean.

## Report (at most 30 lines)
- The rate trial's result and the conditions run.
- A table per condition: owner, p50, p99, p99.9, max, count, loss, failures, and the verdict.
- Cell metrics.
- The fault runs.
- Host, load and wall times.
- Rows added.
- Size in lines.
- Anything in this brief you think is wrong.

## Amendments after Sol's check of this brief (2026-11-02; they override the sections above where they conflict)
Sol's findings 1–7 (`sol-e2b-brief.md`) are all adopted.

1. **Two delays per update.**
   - **Entry delay:** apply entry minus the host instant of `e − window`. A diagnostic.
   - **Acceptance delay:** the host instant at which the timed command is in the device's queue (`apply` returned `Ok`) minus the host instant of `e − window`. **This one is binding for the verdict.** It covers the waits inside the device call (the FakeDevice state mutex, a full queue).

   The DSP-log check (taken at tick `e`) stays a separate pass/fail per update, matched one-to-one with the table by `(tick, frequency, timed)`, never by when the log was read.
2. **Ingress schedule** (fixed, the same for both owners, recorded):
   - each update enters its owner at `e − 50 ms`, in order;
   - at most 8 updates are booked and not yet applied at any time (bounded lookahead), so today's owner never meets its 16-command limit and the booking floor never moves an update by design.

   Record each update's requested instant, effective instant and ingress lateness. An update that today's owner moves or refuses under this schedule is a failure, and it is reported with the cause. The new runtime takes its updates through the same ingress and holds them until its release instant. It does not bypass booking: give it the same `timed_instant` floor (`provider/control.rs:641-673`) or show that the schedule never reaches it.
3. **Equal receive work.**
   - Both owners use the same receive length per call (`block_len`) and the same packet policy (`rx_packet`).
   - Both publish real payloads: the new runtime's `R` converts the samples and hands each block to a sink that does work equivalent to today's owner's per-block publication (cf32 conversion and a `SampleBlock`, or an equal-cost stand-in shown with a measured equal cost). The sink is consumed by a reader thread with a bounded queue, the same for both owners.

   State the per-block work of each in the report.
4. **Rate trial.**
   - Fix the candidates before running: 200 MS/s × 2,000, then 100,000 blocks/s at `block_len` 1,000 / 500 / 200 with the matching decimated rate. Verify each actual rate after decimation.
   - A candidate passes only if a run as long as the measurement keeps the achieved blocks per second within 1% of the target, and the fake's backlog does not grow over the run (sample it in late-run windows).
   - If none passes, the 100,000 blocks/s condition is reported **unsupported**, and the verdict rests on the highest sustained block rate plus 1 MS/s. State which.
5. **Instrumentation runs apart from verdict runs.** Cell hold and wait timing, and any per-block timestamping, run only in separate diagnostic runs. Verdict runs keep counters only (retries). Report one paired on/off run per owner to show the counters' overhead.
6. **Loss in comparable units at a common endpoint.** Report three numbers separately, over the same observation window for both owners:
   - overflow events;
   - missing samples (the gap test);
   - publication drops (blocks produced by the device but not delivered to the sink reader).

   The new runtime must not exceed today's owner in any of them.
7. **The release timer.** The new runtime's release input is delivered by a dedicated deadline mechanism: a timer thread that sleeps until the earliest pending release and is woken when a new release is booked (a condvar with a timeout at the deadline, not a fixed 2 ms tick). Its lateness is inside the measured delay. No early delivery, and no synchronous delivery by the harness thread.

Also:
- **Interleave counterbalanced:** ABBA, then BAAB, with per-run results kept.
- **p99.9:** nearest rank, zero-based index 9,989 of 10,000, per condition and per repetition.
- **Host/device clock offset:** establish it after the time-zero setup, from `time_now` brackets, and report its uncertainty.
