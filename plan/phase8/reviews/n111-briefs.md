# #111 + #119 — briefs

## brief-plan

# #111 — cleanup's freeze calls the Authority's cancel() under the admission lock: test plan first (Sonnet; risky step, AGENTS §9 steps 1–2)

Worktree `~/local_work/Ez-SDRv4-n111`, branch `work/n111` (base main `c31a2138`). READ-ONLY in this phase: write nothing in the worktree; your output is the test plan, written as your final message (writing files is refused). Do not commit.

## The step (risky: a lock and a lifecycle across a supplied callback)
Read `gh issue view 111 -R k3komatsu/Ez-SDRv4`. RS-6 step 1, `CleanupStep::FreezeDispatch` (`crates/ezsdr-kernel/src/coordinator/ending.rs` ~291–316), holds the admission lock while it calls `self.shared.cancel(handle)` for every scheduled wake-up — a call into the supplied `TimeAuthority`. A Provider worker holding a mutex the Authority's `cancel` needs and calling `actions_out.submit` (which takes the admission lock) deadlocks against it. #103 (records `plan/phase8/reviews/n103-*`) moved the start's clock reads out of the lock for the same reason (KC-24a; `Shared::sample_transition`, `RunStateMachine::move_to_at`, the `testing`-gated `coordinator::start_probe` seam). Fix direction (the issue's): drain `scheduled` and the wake handle under the lock, cancel them after dropping it, since the freeze already makes the handles unusable for dispatch.

## What the plan must contain
1. Your own failure scenarios first (before the fix direction): every callback into a supplied object (TimeAuthority `now`/`cancel`/`schedule`, a Provider's or Module's hooks) that ending/cleanup makes while holding the admission lock or another lock a Module thread can need, with a minimal operation sequence for each; whether any other `ending.rs`/`pipeline.rs` path has the same shape (search and list with path:line; the step fixes only what the brief names unless you find the same mechanism — then list it as a proposal).
2. The operation sequences that matter, each with its expected result: the deadlock sequence above (must complete after the fix); a wake-up that fires between the drain and the cancel (must not dispatch: why); an end requested twice; cleanup racing a late submission (KC-24a's existing guarantees unchanged); the record (`termination.undelivered`, the log) unchanged.
3. For each sequence: the test (name, file), how it forces the interleaving deterministically (reuse #103's seam pattern or the existing doubles; no wall clock, no sleeps as oracles), what it asserts, the assertion that proves the interleaving was reached, and the smallest wrong change it must catch (failing on that assertion, not on a timeout). The worst descheduling point per thread (`~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`).
4. Mutation rows (Rust token anchors, smallest unique text, killing test named).
5. Spec texts that change (KC-24a's Ceiling, RS-6 step 1) and the exact new sentences.
6. Open questions with your recommendation each.

## brief-fix1

# #111 + #119 fix round 1 (after the Opus + Terra sub-step review) — Sonnet drives Go DeepSeek

Worktree `~/local_work/Ez-SDRv4-n111`, branch `work/n111` at 531c73a3 (your implementation, committed by the orchestrator). Work only there; never touch `~/local_work/Ez-SDRv4` (another lane). Do not commit, stash, push, reset or switch branch. Process: DeepSeek tasks `dsf-<n>-task.md`, logs `dsf-<n>.log` in `~/.cache/ezsdr-fixes/2026-11-19-n111/` (a task file must not start with `---`). Launch: `cd ~/local_work/Ez-SDRv4-n111 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`; real PID via `pgrep -n -f 'deepseek-v4.1-flash#max'`, check `/proc/<PID>/cwd`. **Wait with the Bash tool's `timeout` set to 600000**: `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`. Every task tells DeepSeek: its shell cuts commands at 120 s (cargo timeout ≥ 1800 s); never `mutate.py`, never a mutant or wrong change in the tree (probes in a /tmp copy with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`); no commit; no files outside the task. Rows: `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py phase7 sim02:7,sim03:7 <ids>` from the worktree (`chmod 644` edited files first; the queue may be held by a full run — wait). Tests run locally (the worktree's own `.cargo/config.toml` target). At most two send-backs.

Fix exactly the adopted findings in the last section of `~/.cache/ezsdr-fixes/2026-11-19-n111/decisions.md` (reviews: `opus-code.md`, `terra-code.md` there). No production behaviour change: the production code was found correct; only test hooks, tests, rows and spec text change (a test-only hook point in production code under the existing `testing` hooks is fine). Each new or retargeted row must die by assertion; show it. A full mutation run of 531c73a3 is in progress (`full.log`); do not start another full run.

Exit checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (state the result of `t200_bare_uhd_script_judges_a_run_and_bounds_it` in the foreground), the changed tests ×20, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base c31a2138`. Adversarial review: `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.

Report (Japanese, short): `git diff --stat`; per finding the fix, test and row with outcome; checks; deviations; INFERRED.

## brief-fix2

# #111 + #119 fix round 2 (after Sol's code review) — Sonnet drives Go DeepSeek

Worktree `~/local_work/Ez-SDRv4-n111`, branch `work/n111` at 099fd26c (implementation + fix round 1, committed by the orchestrator). Work only there; never touch `~/local_work/Ez-SDRv4` (another lane). Do not commit, stash, push, reset or switch branch. Process: DeepSeek tasks `dsg-<n>-task.md`, logs `dsg-<n>.log` in `~/.cache/ezsdr-fixes/2026-11-19-n111/` (a task file must not start with `---`). Launch: `cd ~/local_work/Ez-SDRv4-n111 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`; real PID via `pgrep -n -f 'deepseek-v4.1-flash#max'`, check `/proc/<PID>/cwd`. **Wait with the Bash tool's `timeout` set to 600000**: `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`. Every task tells DeepSeek: its shell cuts commands at 120 s (cargo timeout ≥ 1800 s); never `mutate.py`, never a mutant or wrong change in the tree (probes in a /tmp copy with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`); no commit; no files outside the task. Rows: `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py phase7 sim02:7,sim03:7 <ids>` from the worktree (`chmod 644` edited files first; the queue may be held by a full run — wait). Tests run locally (the worktree's own `.cargo/config.toml` target). At most two send-backs.

Fix exactly the adopted findings in the last section of `~/.cache/ezsdr-fixes/2026-11-19-n111/decisions.md` (review: `sol-code.md` there). F1 changes production (test-first: Sol's sequence fails before the fix); F2–F4 change tests, test hooks and field visibility only. Each new or retargeted row must die by assertion; show it. Do not start a full mutation run (the orchestrator runs it at the gate); run the rows you touch or add plus N111-*.

Exit checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (state the result of `t200_bare_uhd_script_judges_a_run_and_bounds_it` in the foreground), the changed tests ×20, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base c31a2138`. Adversarial review: `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.

Report (Japanese, short): `git diff --stat`; per finding the fix, test and row with outcome; checks; deviations; INFERRED.
