# Tools step (lane 2, before Gate X): note 52 D2 (feature `uhd` in the mutation gate) and D1's trial of a mutant generator — Sonnet, may drive Go DeepSeek

Worktree `<worktree>`. Create branch `work/tools2` from `main` (`3928407f`) with `git switch -c work/tools2 main` — the only branch operation you may do. Then do not commit, stash, push, reset or switch branch. Never touch `<main tree>` (another lane). Work dir for logs and notes: `~/.cache/ezsdr-fixes/2026-11-20-tools2/`.

**Risk: not risky** (tooling only; no Phase 8 production code changes). No test plan. Terra reviews the final diff (note 52, orchestrator's decision).

## What was decided (read `plan/phase8/52-phase8-end-review.md` §2a and §4 D1, D2; AGENTS.md §7 and the Safety core)
- D2: the mutation image (`plan/maintenance/tools/remote/Dockerfile`, built by `remote_mutate.py` on each host) is rebuilt on top of `ezsdr-v4-dev:uhd4.10` (`docker/uhd4.10/Dockerfile`; present on sim02 and sim03), so rows can build `ezsdr-radio-uhd` with `--features uhd` against libuhd. A row may carry an optional `features` field (a list of cargo features), passed to the row's build and test. Which `uhd.rs` rows to keep is decided by D1's trial.
- D1: hand-written rows stay the gate. A generator (cargo-mutants) is only a sweep of production code before the v4.0 freeze; this step is its **small trial**: `ezsdr-radio` and `crates/ezsdr-radio-uhd/src/uhd.rs` (with feature `uhd` and libuhd). The trial decides: adopt if the whole workspace can be swept overnight on sim02+sim03 and its survivors sorted; otherwise "no generator" goes on the freeze checklist. A generator's sweep runs only in a sim container on the tool's own copy of the tree, never in a working tree (AGENTS Safety core, already written).

## What to do
1. **Image.** Change the mutation Dockerfile to `FROM ezsdr-v4-dev:uhd4.10`. Keep what the current one guarantees: a non-root user (a G20 test passes as root; the dev image runs as root by default), Rust stable for `mutate.py`'s `cargo +stable`, python3 with numpy, rsync, `/src` workdir. Check `remote_mutate.py`'s self-test and `--e2e` cases still pass in the new image, and that a plain (no-feature) row still builds and runs. Install cargo-mutants at a pinned version in the image only if the sweep (step 3) needs it there.
2. **`features` on rows.** `mutate.py` (and `anchor.py` / `rules.py` if they validate row fields) accept an optional `features` list and pass `--features <comma list>` to the row's cargo build and test. Self-test case for it. Add at least one real row on `crates/ezsdr-radio-uhd/src/uhd.rs` with `"features": ["uhd"]`, killed by assertion by a test that runs with that feature (gate's `uhd-test` runs `cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api`; see `plan/maintenance/tools/gate.py:327-350`). Run it through `simq` + `remote_mutate.py`.
3. **D1 trial.** A runner that sweeps cargo-mutants on sim02/sim03 in a container on its own copy of the tree (reuse `remote_mutate.py`'s copy, build and per-shard isolation; do not write a second copier unless that is clearly smaller). Run it under `~/.cache/ezsdr-fixes/tools/simq` (another lane uses the sims; never overlap a gate or a full mutation run). Sweep (a) the `ezsdr-radio` crate, (b) `crates/ezsdr-radio-uhd/src/uhd.rs` with `--features uhd`. Use cargo-mutants' own sharding and timeouts; keep per-test wall-clock bounds in mind (AGENTS §7: shards × CPUs within 32 cores per host).
4. **Trial report** (`~/.cache/ezsdr-fixes/2026-11-20-tools2/trial.md`, Japanese): mutants generated, caught / missed / unviable / timeout per target, wall time and cores used, the extrapolation to the whole workspace (production code only: count mutants with `cargo mutants --list` over the workspace, and scale by the measured per-mutant time) on sim02+sim03 overnight (yes/no, with the arithmetic), and a sorted sample of the missed mutants (at least 10, each: add a test / equivalent with a reason / accept). Recommend which `uhd.rs` missed mutants deserve hand-written rows (D2's question) — do not add more than the one demonstration row unless they are clearly worth it.
5. AGENTS.md §7's remote-mutation paragraph: one sentence that rows may carry `features` and the image is built on `ezsdr-v4-dev:uhd4.10`. handoff is the orchestrator's.

## Rules
- Mutation rows run only through `remote_mutate.py` (via `simq`); never run `mutate.py` locally or apply a mutant to a tree. The cargo-mutants sweep runs only in a sim container on a copy.
- `REMOTE_SSH="ssh -o BatchMode=yes"`, `PYTHONDONTWRITEBYTECODE=1`. Build output never in the tree; test temp files on /tmp. Kill processes by PID only. Never `chmod -R`; chmod 644 files you create in the tree.
- DeepSeek, if you use it: tasks `ds-<n>-task.md` and logs `ds-<n>.log` in the work dir; a task file must not start with `---`; launch `cd <worktree> && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`; real PID via `pgrep -n -f 'deepseek-v4.1-flash#max'`, check `/proc/<PID>/cwd`; wait with `timeout 590 tail --pid=<PID> -f /dev/null` (Bash timeout 600000); tell it the shell cuts at 120 s and not to commit. Review its diff yourself (at most two send-backs).
- Before reporting: `cargo fmt --all --check`, `python3 plan/maintenance/tools/rules.py --base 3928407f`, `python3 plan/maintenance/tools/rules.py --self-test`, mutate.py's self-test (in the image, via remote_mutate.py's self-test path). No full mutation run (the orchestrator's gate does it).

## Report
Japanese, short: diff stat; the image change and what you checked in it; the `features` field and the demonstration row's verdict; the trial's numbers and the overnight answer; the recommended `uhd.rs` rows; INFERRED items; open questions with your recommendation.


## brief-fix

# Tools step fix round — Sonnet, implement directly

Worktree `<worktree>`, branch `work/tools2` at `9db289dc` (your work, committed). Same rules as `brief.md` beside this file. Do not commit, stash, push, reset or switch branch.

Terra's review: `~/.cache/ezsdr-fixes/2026-11-20-tools2/terra.md`. Dispositions (orchestrator):

1. **Terra 1 (major, shared image tag): adopted.** `remote_mutate.py` and `sweep_mutants.py` build and run under a per-run tag (e.g. `ezsdr-mut-<the run's work-dir tag>`), removed at the end of the run like the work dir; no run uses a tag another checkout can rebuild. Keep the e2e/self-test paths on the same per-run tag. Check `gate.py` does not name `ezsdr-mut` itself (if it does, follow it).
2. **Terra 2 (major, a partial sweep reports success): adopted.** The sweep waits for each container's exit code, requires one complete result per requested shard, refuses stale results (fresh output directory per run, or refuse a non-empty one), and exits non-zero on any failed or missing shard.
3. **Terra 3 (major, timeouts counted as survivors): adopted in the report.** In `trial.md`, the 38 workspace-test mutants stopped at 400 s are "unresolved (timeout)", not survivors, everywhere they are classified; say what a complete run would need (the per-mutant timeout above the ~550 s suite).
4. **Terra 4 (minor, the overnight conclusion is conditional): adopted in the report.** State the estimate as ≈14.7 h under the stated assumptions, and the break-even (a first-pass survivor rate ≤ ≈11.5 % fits 12 h), and which measurements would settle it.
5. **Terra 5 (minor): adopted.** `sweep_mutants.py` checks the remote commands' return codes (baseline included) and fails loudly; its docstring says it must run under `simq` (do not build a lock into it).

Run: `remote_mutate.py`'s self-test and one plain row and TL2-1 through `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:1,sim03:1 <ids>`; a tiny sweep (a handful of mutants, e.g. one file of `ezsdr-radio` with a `--file` filter) under `simq` to prove the exit-code and per-shard checks, plus one run where a shard is made to fail (e.g. a bogus filter on one shard) to show the non-zero exit. Logs in the work dir. `python3 plan/maintenance/tools/rules.py --base 3928407f` and `--self-test`. No full mutation run. Remove any temporary image you create on the sims.

Report (Japanese, short): diff stat; each item; the runs.


## brief-fix2

# Tools step fix round 2 — Sonnet, implement directly

Worktree `<worktree>`, branch `work/tools2` at `cfd361b7`. Same rules as `brief.md` / `brief-fix.md` beside this file. Do not commit, stash, push, reset or switch branch.

Terra's re-check: `terra-recheck.md` beside this file. All adopted (orchestrator):
1. **Unique run id**: `remote_mutate.py` and `sweep_mutants.py` derive the run's tag (work dir and image) from a random id (e.g. `uuid4().hex[:12]`, optionally with the time), never `int(time.time())` alone; the same for e2e, list and baselines.
2. **Cleanup on every exit path**: track what the run created on each host (uploaded dir, image tag, containers) and remove it in a `finally` (best effort, errors printed, the original exception kept), in both scripts including e2e.
3. **Per-shard completeness**: each requested shard's `outcomes.json` must parse and hold at least one mutant outcome (or exactly the mutants that shard was given, if cargo-mutants' sharding makes that knowable); a shard that fails this is reported in the same FAILED list as a missing file, malformed JSON included.
4. **trial.md**: line ~14 and line ~69 — the 400 s cases are unresolved (timeout), not survivors and not "no workspace test".

Runs: the e2e and one row through `simq` + `remote_mutate.py`; a tiny sweep under `simq` (as before) and one where one shard is made to return an empty result (e.g. a filter matching mutants only in one shard's slice, or a test hook) → non-zero exit. Show in the log that no image, dir or container is left on the sims after a run that raised (e.g. kill the ssh of a run mid-way, or force an exception with an invalid argument after upload). `python3 plan/maintenance/tools/rules.py --base 3928407f`, `--self-test`. Logs in the work dir.

Report (Japanese, short): diff stat; each item; the runs.


## brief-fix3

# Tools step fix round 3 (last) — Sonnet, implement directly

Worktree `<worktree>`, branch `work/tools2` (fix round 2 committed; the orchestrator has added uncommitted record files under `plan/phase8/` and edits to `AGENTS.md`, `handoff.md`, `plan/phase8/50-remaining-work.md`, `plan/phase8/52-phase8-end-review.md` — do not touch those). Same rules as `brief.md`. Do not commit, stash, push, reset or switch branch.

Terra's re-check 2: `terra-recheck2.md` beside this file. Orchestrator: all three adopted, as robustness of a tool (leaked containers or images on the sims), so this is the last round and gets no further review:
1. Cleanup per host never stops the loop: each host's cleanup in its own try/except (BaseException), errors printed, the original exception re-raised after all hosts; the remote cleanup command reports a failed `docker rm -f` / `docker rmi` / `rm -rf` (no blanket `|| true`; a "no such container/image" is fine).
2. Name the `list` and `baselines` containers from the run tag so cleanup finds them.
3. A malformed outcome record (e.g. `scenario` without `summary`) is a shard failure in the same FAILED report, not an exception.

Runs: one tiny sweep under `simq` and one interrupted `list` (Ctrl-C equivalent: kill the local process by PID while the remote container runs) → nothing left on the sims; logs in the work dir. `python3 plan/maintenance/tools/rules.py --base 3928407f`.

Report (Japanese, short): diff stat; each item; the runs.
