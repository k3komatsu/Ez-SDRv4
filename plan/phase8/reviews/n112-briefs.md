# #112 and #114 — briefs

#112 was written by the orchestrator (no brief).

## #114

Worktree `/home/komatsu/local_work/Ez-SDRv4-n112`, branch `work/n112` (base main `6cc8026d`; HEAD `5cbccee9` holds #112, a Python fix — do not touch it). Work only there; never touch `/home/komatsu/local_work/Ez-SDRv4` (another lane's tree) or other worktrees. Do not commit, stash, push, reset or switch branch. The worktree has its own `.cargo/config.toml` target dir; use it.

## The defect (issue #114, read it: `gh issue view 114 -R k3komatsu/Ez-SDRv4`)
`Violation.requested` means what the experimenter asked ("What was asked." in the schema). At prepare, three producers in `crates/ezsdr-kernel/src/plan/prepare.rs` set `requested: Some(applied.clone())`, a Module's reported value: MA-12's narrowing check (the `node.capabilities` loop, `check: ezsdr.effective`, reason `e.message`), SB-44's preview mismatch, and MA-12's re-match (`MA-12 (re-match)`). The step that landed as `6cc8026d` fixed the same defect for the SB-6 kind violations with `coercion::exact_request(req, key)` (the Spec's `Eq` value as a `Value`, else `None`) and put the reported value in `reason` (see `prepare.rs:~112`, `matching.rs:~60`, and SB-6 in `design/03-spec-and-binding.md`).

## What to do
- The three producers use `exact_request` (the resource's request is in scope; check), and each reason names the reported value (`{applied:?}`) where it does not already. One rule for every Violation producer at prepare and validate: after the change, search `crates/ezsdr-kernel/src` for every `Violation {` and list each one's `requested` source in your report (a registered check's own `requested` is the check's business, not this fix's).
- Tests: change any test that pins the old record; add one test per producer that fails on the current code at its `requested` assertion (`requested` is the Spec's `Eq` value; absent for a range/set/unnamed key) and checks the reason carries the reported value. One mutation row per producer (restore `Some(applied.clone())`), killed by its test.
- Texts: MA-12 and SB-44 in `design/03` (and wherever `Violation.requested` is described for these checks) say what `requested` holds, in the SB-6 wording; their `*Test:*` lines name the new tests; a Changes row. No issue numbers in rule text (AGENTS §2).

## Process
- Write the production lines into DeepSeek's task yourself (they are a few lines); DeepSeek writes the tests, rows and runs the checks. Task files `ds-114-<n>-task.md`, logs `ds-114-<n>.log`, rows `rows-114.log` in `/home/komatsu/.cache/ezsdr-fixes/2026-11-19-n112/`. Launch DeepSeek detached from the worktree: `cd /home/komatsu/local_work/Ez-SDRv4-n112 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`; take the real PID with `pgrep -n -f 'deepseek-v4.1-flash#max'` and check `/proc/<PID>/cwd` is this worktree (another lane runs DeepSeek in the main tree). **Wait with the Bash tool's `timeout` parameter set to 600000**: `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`. Every task tells DeepSeek: its shell cuts commands at 120 s, so cargo gets a timeout ≥ 1800 s; never `mutate.py`, never a mutant or wrong change in the tree (wrong-change probes in a /tmp copy built with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`); no wall-clock tests; no commit; no files outside the task.
- Rows through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:4,sim03:4 <ids>` (Bash `timeout` 600000, or `run_in_background`): the new rows plus every row whose filter selects a test you changed. `chmod 644` new or edited files first.
- At most two send-backs. Your review is adversarial: `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.
- Exit checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `schema_freeze` (no schema should move; if a description changes, regenerate with `EZSDR_UPDATE_SCHEMAS=1` and list them), `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 6cc8026d`.

## Report
Final message: `git diff --stat` against `5cbccee9`; the producers' list with each `requested` source; each test and row with its outcome; check counts; the adversarial review one line per test; the trial record; deviations and INFERRED.

## Extension and fix round (messages to the same Sonnet subagent)

1. Extend to every producer still recording a Module's `Coercion.requested` (SB-46 at validate and prepare, the SB-44 stray-key sites); one rule: what the experimenter or the Action asked, else none; the reported value in the reason.
2. Fix Sol's F1–F4 as `decisions` disposes them (Sink `params` lookup; closed check before encoding; explicit exception type; tests merged into the SB-44 stray tests).
