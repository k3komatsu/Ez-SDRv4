# Brief — step C: three load-sensitive tests no longer depend on host wall-clock timing (#83)

Repository /home/komatsu/local_work/Ez-SDRv4, branch `work/stepC` (base `main` at the commit the orchestrator names; clean). Read AGENTS.md "Safety core", §7, §9 (you are the implementer). Issue: `gh issue view 83`. Mechanism: `plan/maintenance/failure-mechanisms.md` row `mech:wall-clock-test-oracle`; the earlier fix pattern is plan 30 (`plan/maintenance/30-fakedevice-manual-time.md`, the manual host clock for the FakeDevice smoke set).

**Risk: not risky** (tests only; no production code, no design text unless a test's cited name changes). No test plan review; one cheap review at the end; the orchestrator runs the gate.

## The three tests

1. `crates/ezsdr-acceptance/tests/parity.rs` `s15_parity_untimed_booking_after_stop` (~:2183). It asserts the untimed repeat after a Stop starts 9 900..10 100 samples after the Stop's sample on both Providers, but takes the UHD Stop's sample from the *submitted* tick (`pair.uhd_t0 + ms(20)`), while the wall-clock FakeDevice's device clock runs ahead of it (the test's own comment at ~:2216-2217). Under load the gap reached 10 103 and 10 651. Make the verdict independent of host scheduling: compare the repeat's target with the instant at which the UHD Provider actually applied the Stop (from what the run records — e.g. the Stop's bound in the record/Manifest or the cancelled booking's region — find the existing observable; do not add production instrumentation), or run the UHD side of this test on the manual-clock FakeDevice if the pair harness supports it (parity.rs:2257-2258 pins boundaries per Provider for the same reason). The assertion must still fail if the Stop region were not 10 ms (KL-13/KL-15) — say which wrong behaviour still fails it.
2. `crates/ezsdr-radio-uhd/tests/common/measurement.rs` test `rx_start_lead_group_runs_on_the_fake` and `crates/ezsdr-radio-uhd/tests/fake.rs` `provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements` (≈ 53 s together). They failed as the mutation BASELINE of rows N47B-19, N47B-36 and P824 under 16-shard load and pass alone. First find what in them depends on wall time (read them; reproduce under load remotely, e.g. run the test binary with `--test-threads=16` alongside a CPU hog, or several copies in parallel through `rdev.py`; never locally). Then make their verdicts independent of host scheduling (manual clock, or deadlines derived from the run's own recorded device times instead of wall-clock bounds), keeping each one the killer of its mutation rows (N47B-19, N47B-36, P824: read their `old`/`new`/`filter` in `plan/maintenance/tools/mutations.json`).

## Rules (binding)

- Change only tests and test support (and a mutation row's `old` if a re-anchor is unavoidable; say so). No production code.
- Do not commit, push or `git add`. Do not run mutate.py or remote_mutate.py; never apply a mutant to the tree (a debug copy outside the repository is allowed).
- Builds and tests only through `python3 plan/maintenance/tools/remote/rdev.py -- <cargo …>` in the foreground until each ends. No cargo locally except `cargo fmt --all`.
- Before reporting: `cargo fmt --all`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base <base>` = 0.
- Evidence of the fix: each changed test passes alone and under a load that made the old version fail (show both, with the command); report how many repetitions (20 for each, since their verdict depended on timing — AGENTS §9 step 8).
- Do not edit `handoff.md`, `AGENTS.md`, `plan/phase8/49-*.md`.

## Report (reply text)

Per test: what depended on wall time (path:line), the change, the wrong behaviour that still fails it, the load reproduction before/after (commands and counts), and the rows it still kills (by reasoning; the orchestrator runs them).

## Owner decision (2026-11-09, during the step)

Test 1 cannot be fixed in tests alone: the instant the UHD Provider applied the Stop is not recorded anywhere. Option (a) adopted: uhd-control records the applied transmit Stop (instant and bound) as a Manifest timing row; the parity test compares against it exactly. One production line; no behaviour change. Rejected (b): widening the UHD band by 1 ms (weaker test, may still flake).

## Sol review (2026-11-09): FAIL, 1 × P2, 1 × P3 — adopted
- P3 fixed: `advance_until` bounds an absolute device-time deadline.
- P2 (the parity test still depends on the test thread between the Stop and the repeat's receipt): the receipt instant is not observable in the parity harness. Owner decision (B): no production change; the parity test asserts the UHD Stop region (10003..10004 from `tx_stop_applied`) and that the repeat starts no earlier than the bound on both Providers; the exact placement is pinned per Provider by manual-clock tests (a uhd-control unit test fixes the receipt instant). Rejected: (A) receipt timing rows in both Providers; (C) device-time interval around the push; (D) route through the Kernel.
