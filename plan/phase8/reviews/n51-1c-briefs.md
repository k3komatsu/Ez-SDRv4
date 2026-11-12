# Note 51 stage 1c — briefs (2026-11-12)

## brief-plan

### Brief — note 51 stage 1c, phase 1: the test plan (no code)

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1c` from `main` `6301ddc`. Read only; write only the plan file below. You are the implementer (OpenCode Sonnet 5.5 #high); stage 1c is **risky**: the test plan comes first and Sol reviews it.

Read first: AGENTS.md §3, §7, §9; plan/phase8/51-test-timing.md (§3, §4.1–§4.2, §5, §7.1, §7.2 rows 1b–1d and the paragraph "段 1b の後" at the end, §8, §9's line on note 37 decision 4, §11, the appendix decisions incl. Q6); plan/phase8/51-tables.md (the W table and every row for `Direct` in `crates/ezsdr-radio-uhd/tests/fake.rs`, §2.3, §4); the 1a and 1b records plan/phase8/reviews/n51-1a-* and n51-1b-* (the `HostClock::drive()` / `Driver` API; how `UhdRig` registers its thread, why `wait_until` ends 1 µs past its tick, the drop-order tests and their ordering-fact verdicts); crates/ezsdr-radio-uhd/tests/README.md (the `Direct` exception that 1c removes); crates/ezsdr-radio-uhd/tests/fake.rs (`Direct` and its 38 tests), crates/ezsdr-radio-uhd/src/host.rs, authority.rs, device.rs.

## Stage 1c (note 51 §7.2: "`Direct`：W 表のとおり，待ちの移行と登録を同じ commit")

`Direct` (the Provider-direct rig in `fake.rs`) drives its manual clock the way `UhdRig` does since 1b: its thread registers as a participant; every wait it does moves to the clock as the W table says, in the same commit as the registration (a registered thread that waits off the clock freezes device time forever). Apply 1b's lessons: receipts at a fixed instant (the 1 µs rule), drop order pinned by an ordering fact, no verdict from transient counts, bounds that write the infra marker. The README's `Direct` exception goes away.

## The plan (write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1c/test-plan.md)

1. `Direct`'s waits, each from the W table with path:line, and what it becomes; any wait the table missed (search and give the search).
2. The 38 tests: which depend today on when the test thread runs (with a diagnostic delay injection in an out-of-tree copy, note 51 §5.4: diagnostic only), which change, which are unaffected; the failing-first set.
3. The rig changes, each with its test (registration position, drop and unwind order, two rigs, waits on the clock, the receipt instant, the bounds' markers).
4. Mutation rows (Rust token anchors), each naming a killer that fails on an assertion, not on a bound or a race; existing rows whose filter or killer changes.
5. Text 1c makes false (the tests README, UR rules in design/18, note 51, the tables).
6. Sub-steps, if any, each leaving the tree green.

Rules: no code edits in the tree; probes only in a copy outside the repository built **locally** (`rsync -a --no-times --exclude .cargo` to /tmp/opencode/<name>, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`), not on sim02/sim03; no commit. Final message: a summary, and anything in note 51, the tables or the 1a/1b records the code contradicts.

## brief-impl1

### Brief — 1c-A: amend the plan for Sol's review and the decisions, then sub-step A

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1c` at `6301ddc` (the step's start). You are the stage 1c implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1c/: brief-plan.md, test-plan.md (yours), plan-decisions.md (all of it, including "After Sol's plan review"), sol-plan.md (Sol's plan review: FAIL, three majors, all adopted).

## Amend the plan first (a section "Changes after Sol's review and the decisions" at the top of test-plan.md)

Close M1, M2 and M3 as plan-decisions.md says, with sub-step C (one shared drop-order guard for both rigs) adopted, D deferred, and the missing device-time horizon on `UhdRig::wait_for_events` added in sub-step A. M3's rule covers `UhdRig` too (stage 1b's `wait_until` epsilon becomes the shared rule; keep 1b's tests green with unedited assertions, or say exactly why one must change).

## Then: sub-step A (plan §6 as amended): waits and bounds move, no registration yet; the shared wait-return rule (M3) where it does not need registration, else in B.

## Rules

Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). **Build and test locally** (the main tree's `.cargo/config.toml` target), not on sim02/sim03. Probe copies rebuilt from source every time (M2). Run the rows you add or touch through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` and wait for it to finish before reporting. Temp files on /tmp; kill by PID only; never a mutant in the tree. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 6301ddc` = 0. Timing-dependent tests 20 runs, T4 96 runs under load. No commit.

## Report

`git diff --stat`; the plan amendments; per test: what it pins, runs; rows and outcomes; contradictions with the plan.

## brief-implB

### Brief — 1c-B (see test-plan.md §6 as amended, sub-step B)

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1c` (step start `6301ddc`; earlier sub-steps are local commits — `git log 6301ddc..HEAD`). You are the stage 1c implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1c/: brief-plan.md, test-plan.md (all its "Changes" sections), plan-decisions.md, sol-plan.md, brief-impl1.md and the previous sub-steps' briefs. Do **only sub-step B** of the amended plan, exactly as it states (content, tests, rows, text).

Rules: tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3); existing assertions unedited unless the plan names the change. **Build and test locally**, not on sim02/sim03; probe copies rebuilt from source every time. Run the rows you add or touch through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` and **wait for it to finish before reporting**. Never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 6301ddc` = 0. Timing-dependent tests 20 runs, T4 96 runs under load. Add a "Changes in 1c-B" section to test-plan.md. No commit.

Report: `git diff --stat`; what the sub-step required and what you did; rows and outcomes; deviations with reasons.

## Orchestrator decisions after 1c-A (2026-11-12)
- 1c-A's contradiction 2 is accepted: R1 (the 1 µs past the chosen instant) applies only to waits that end on an instant the test chose (`settle`, `wait_until`); waits that end on an observed state (`UhdRig::wait_for_events`, and in B `Direct::until`) do not take R1. Treat `Direct::until` the same way, and say so in the README rule text.
- 1c-A's contradiction 1 (three 1b assertions now expect 7 ms + 1 µs) is accepted; 1b's exact-duration claim in note 51 is replaced by the one rule (record it in "Changes in 1c-B" so the records match).
- Re-run N47S3-37 and P473 on your final tree with your rows and report the failing assertion (test name and message) for each row you add or touch.

## brief-implC

### Brief — 1c-C (see test-plan.md §6 as amended, sub-step C)

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1c` (step start `6301ddc`; earlier sub-steps are local commits — `git log 6301ddc..HEAD`). You are the stage 1c implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1c/: brief-plan.md, test-plan.md (all its "Changes" sections), plan-decisions.md, sol-plan.md, brief-impl1.md and the previous sub-steps' briefs. Do **only sub-step C** of the amended plan, exactly as it states (content, tests, rows, text).

Rules: tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3); existing assertions unedited unless the plan names the change. **Build and test locally**, not on sim02/sim03; probe copies rebuilt from source every time. Run the rows you add or touch through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` and **wait for it to finish before reporting**. Never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 6301ddc` = 0. Timing-dependent tests 20 runs, T4 96 runs under load. Add a "Changes in 1c-C" section to test-plan.md. No commit.

Report: `git diff --stat`; what the sub-step required and what you did; rows and outcomes; deviations with reasons.

## Orchestrator notes after 1c-B (2026-11-12)
- 1c-B left the drop-order guard as a copy in `fake.rs` (deviation 2). If your row moves both rigs onto one copy in `crates/ezsdr-radio-uhd/tests/common/rig.rs`, do it as a pure move (no behaviour change), re-anchor the rows it breaks to the same program, and run them.
- `Direct::until` and `UhdRig::wait_for_events` take no R1 (decided after 1c-A); keep that.

## Orchestrator decisions on the plan

### Stage 1c — orchestrator decisions on the plan (2026-11-12, decided by the orchestrator)

- Sub-steps A (waits and bounds move, no registration) and B (the driver) as planned. **C adopted**: one shared drop-order guard for `UhdRig` and `Direct` (one copy of the decision; AGENTS §9 maintainability 1). **D deferred** to stage 3 (the real-clock `Gated` waits are a real-clock classification question, not 1c's).
- The missing device-time horizon on `UhdRig::wait_for_events` (1b; note 51 §6.5) is added in sub-step A with `Direct`'s, the same shape and marker.
- The unexplained T4 race (14/96 and 10/96 in one sketch build, receipts 1 ms apart, 0 failures in eight later builds) is **not accepted as noise**: before sub-step B is committed, its cause must be found or the race shown impossible by construction. Sol's plan review is asked to examine it; the implementer runs T4 96 times under load at each sub-step and stops and reports on any failure, keeping the failing binary.
- Counts corrected in the records at landing (41 `Direct` tests, 416 in `fake`); the README exception and note 37 decision 4 removed with the code.

## After Sol's plan review (sol-plan.md: FAIL, three majors) — all adopted (decided by the orchestrator)
- M2 explains the T4 race: the binary was built with M12's bypass (`Direct::start` calling the raw `wait_until`, no trailing 1 µs); not a stage-1a clock defect. Make the `start()` duration assertion and M12's coverage mandatory; rebuild and verify a baseline before collecting probe evidence (a probe copy is rebuilt from restored source, never reused).
- M3: a fixed epsilon is not a general rule (`settle(999 µs)` from an armed rig ends on a poll instant). Replace per-method epsilons by **one rig-level rule for every wait that returns control to the test (`wait_until`, `settle`, `until`…, on `Direct` and on `UhdRig`)**: before returning, every participant due at the instant the wait ended has run (for example one shared helper that ends each wait with a clock wait past the instant, or an equivalent ordering established at the final wake), stated once in the README; add the `settle(999 µs)` boundary case for both rigs. If no general rule is possible within the rig, constrain and document the supported phases and say why.
- M1: W13's rewrite must keep the ordering P473 tests: an entry/release handshake observable while the owner is held in `apply`, an explicit assertion that the Stop happened during the configuration, and the infra marker on a setup timeout; P473 re-run and its kill path checked.
