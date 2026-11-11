# Brief — #96: the s15 paired parity tests must not depend on uhd-tx's thread timing

Repository /home/komatsu/local_work/Ez-SDRv4, branch `work/n96` from `main` `fe86629`. Read first: AGENTS.md (§7, §9 "Maintainability", the mutation and build rules), GitHub issue #96 (`gh issue view 96`), #87 (`gh issue view 87`, and how O1 fixed it: `o1_wait_playing` and `o1_uhd_at` in `crates/ezsdr-acceptance/tests/parity.rs` ~:2585), plan/maintenance/failure-mechanisms.md's `mech:wall-clock-test-oracle` row, and plan/phase8/37-test-thread-timing.md (the rule: a test's verdict never depends on when the test thread runs). The failing output of the first #89 gate is in /home/komatsu/.cache/ezsdr-fixes/2026-11-11-n89/gate-fail1/test.log (lines ~60–145).

Risk: not risky (tests only; no production code changes). No test plan and no self-review; the orchestrator reviews, then the full mutation lists, then Sol's code review.

## Goal

The five tests of #96 (`s15_parity_switch_to_a_new_length`, `s15_parity_second_pending_switch_is_refused`, `s15_parity_provider_stop_cancels_a_pending_switch`, `s15_parity_switch_starts_on_the_old_repeats_end`, `s15_parity_untimed_queue_behind_a_booking`) pass whatever the host load, with their assertions unchanged in meaning.

1. **Reproduce first.** Under CPU load inside the rdev container (e.g. as many busy loops as the container has CPUs, or twice that — find the count with `nproc`; note 37 used 16 busy loops on 16 cores), run the parity binary or the s15 tests repeatedly and record how often each fails at `fe86629`. A fix is only shown by a reproduction that failed before it. If you cannot reproduce at all after a reasonable effort (say 20 loaded runs at two load levels), stop and report what you tried.
2. **Diagnose each failing test** from its output and the code: which step lets device time pass a booking's lead, or lets a switch reach the UHD before `uhd-tx` issued the repeat's first send (E2a's (b1) refusal at receipt, KL-41). Say for each test which line of the test and which owner behaviour; mark it VERIFIED (shown by a probe or the reproduction) or INFERRED.
3. **Fix the tests the way #87 was fixed**: wait for observed progress (e.g. the repeat's first timed payload send in `device.calls()`, as `o1_wait_playing` does), then advance or submit. Reuse the existing helpers (`o1_wait_playing`, `o1_uhd_at`, `submit`, `uhd_barrier`) or generalise one; do not add a second copy of the same wait. A bounded wait whose expiry is the failure is fine; a sleep that merely makes the race less likely is not.
4. **Search for the same pattern** in the other `CancellationPair` tests (every test that submits a switch, or a booking whose lead the test thread's `wait_until` could pass before `uhd-tx` acts). Fix those that share the defect; list the ones you checked and why each is safe.
5. **Do not weaken assertions.** The Mock/UHD parity assertions and the expected rows stay. If a test's expected outcome legitimately differs with timing on the UHD (e.g. (b1) refuses a switch the Mock applies when the first send was not issued yet), change the test's sequence so the outcome is determined, not its expectation; if that is impossible, stop and report.
6. **Show the fix**: under the same load, the five tests (and any others you changed) pass at least 20 loaded runs; the unloaded parity binary still passes.

## Rules (binding)

- No production code change (crates/*/src). If one seems needed, stop and report.
- Builds and tests only through `python3 plan/maintenance/tools/remote/rdev.py [--host sim02|sim03] -- <cmd>` (the local PC carries no build load). The shell tool stops a command at 120 s unless you pass a longer timeout: give rdev runs an explicit long timeout (e.g. 3600000 ms) and wait in the foreground until they end; never leave a command running in the background when you report. Kill any busy loops you start (by PID, inside the container) before the command ends.
- Mutation rows: never run mutate.py or remote_mutate.py, never apply a mutant inside the tree. If a row's anchor text or killer test changes, re-anchor it (Rust token anchors; same file and Provider; no two rows identical) and check its killer in a copy outside the repository. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base fe86629` must report 0 problems.
- `cargo fmt --all` before reporting (rustfmt.toml), then `cargo fmt --all --check`.
- Run `rdev -- cargo test --workspace` once at the end.
- Test temp files on /tmp. New files mode 644. Python with PYTHONDONTWRITEBYTECODE=1.
- Do not edit handoff.md, AGENTS.md, KNOWN_LIMITATIONS.md, design/, plan/*/00-overview.md, governance.rs, bench files.
- Do not commit, push or `git add`.

## Report

Write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-11-n96/report.md; your final message is its summary:
- the reproduction (load level, runs, failures per test, before the fix);
- the diagnosis per test (VERIFIED / INFERRED);
- the changes (file:line), the helpers reused or generalised;
- the other tests checked for the pattern, with the verdict for each;
- the loaded runs after the fix, and the unloaded and workspace runs;
- mutation rows touched, with their checked killers;
- anything in this brief that proved wrong.
