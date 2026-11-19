# Fix round 1 of the SB-9a withdrawal (admission-per-origin note): Sonnet drives Go DeepSeek

You supervise Go DeepSeek fixing the findings adopted from Sol 6.2's code review. Worktree `/home/komatsu/local_work/Ez-SDRv4-nsb9a`, branch `work/nsb9a`, HEAD `b38f98cf` (the step, committed locally by the orchestrator; base `7b676bce`). Work only there; never touch `/home/komatsu/local_work/Ez-SDRv4` or other worktrees. Do not commit, stash, push, reset or switch branch.

Read first: `/home/komatsu/.cache/ezsdr-fixes/2026-11-19-admission-note/sol-code.md` (the review), the "Sol 6.2 code review" section at the end of `decisions.md` in the same folder (binding dispositions F1–F6), `plan/phase8/reviews/sb9a-report.md` (what the step did), and `brief-impl.md` in that folder (the step's brief: its "Driving DeepSeek", adversarial review, exit checks and rules sections bind this round too, with the changes below).

## What to fix
F1–F6 as `decisions.md` states them. Design the fix of F1 and F2 yourself and write the key production lines into DeepSeek's task; DeepSeek writes the tests and rows. For F2, check `Violation`'s doc comment and the schema's description of `requested`, and every other producer of a kind violation (`binding::check_value_kind` callers) so all set `requested` by one rule. For F1, name the registered-check call site and make the early return the only change of order. Each F1/F2 fix gets a test that fails on the old code at the named assertion (F1: a registered prepare check that would panic on the wrong kind is never called and the Run ends with the SB-6 violation, not a panic reason; F2: `requested` is the Spec's value or absent, never the reported value) and a mutation row killed by it.

## Changes to the step brief's process
- Task files `ds-fix1-<n>-task.md`, logs `ds-fix1-<n>.log`, rows log `rows-fix1.log`, in `/home/komatsu/.cache/ezsdr-fixes/2026-11-19-admission-note/`.
- **Waiting for DeepSeek or a long command: call the Bash tool with its `timeout` parameter set to 600000** and `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done` — one wait per call of up to ~10 minutes, not 2. `remote_mutate.py` the same way (Bash `timeout` 600000; if it needs longer, `run_in_background` is fine).
- Rows: the rows whose code or killing test this round changes (W9A-1..3, the rows of the tests you touch, any anchor that moved) plus the new ones; not the full lists.
- One send-back at most per run, two in total.
- Tests: no wall-clock verdicts; repeat a test 20 times only if its verdict can depend on timing.

## Report
Your final message is the report: `git diff --stat` (against `b38f98cf`); each finding F1–F6: what changed, the test and the row and its outcome; each check's counts (fmt, clippy, workspace tests, schema_freeze, Python 3.14 suite if anything Python-visible moved, rules.py `--base 7b676bce`); the adversarial review one line per test added or changed; the trial record (DeepSeek runs, durations, send-backs, what you wrote into tasks); deviations and anything INFERRED.
