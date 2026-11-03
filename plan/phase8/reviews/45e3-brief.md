# Brief: note-45 experiment E3 — the holdout and the past-bug table

You are the evaluator for stage E3 of the note-45 experiment. Repository `~/local_work/Ez-SDRv4`, branch `main` (read AGENTS.md first; do not cd into `v3/`).

**You do not edit the main tree.** Every mutated build lives in a copy outside the repository. Make the copy with fresh timestamps (`rsync -a --no-times --exclude .cargo --exclude target --exclude v3 ~/local_work/Ez-SDRv4/ <copy>/`), then `find <copy> -type f -exec touch {} +`, and use `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` with `CARGO_INCREMENTAL=0` (AGENTS.md §7). Never run `mutate.py` on the main tree and never apply a mutant there. Never git commit, push, stash, reset or checkout. Kill processes by PID or container ID only.

## What E3 is (`plan/phase8/45-experiment.md` §2 and §8; note 45 `plan/phase8/45-uhd-decision-core.md` §7)
Note 45 §7 sets the criteria. This stage measures two of them.
- **Independently chosen errors.** Before any code was written, Sol chose 20 behavioural errors from note 45 alone, for the brain and cell procedures. Errors that only disable an oracle are excluded. The DST must find **at least 16** of them **without changing an observable assertion**.
- **The past-bug table.** For the 6 D9b-2 findings and the four L1 findings (M1, M3, the receive window, the wiring omission), put each into one of:
  - "cannot happen in this form (reason)";
  - "found by the DST";
  - "not solved", with its reason.

The holdout is in `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`. **This stage, and only this stage, reads it.** Read all of it first, and do not copy it into the repository.

The code under test is `crates/ezsdr-radio-uhd/src/brain/` at the current tree (E1, E2a and E2b). The DST consists of:
- the default brain suite (`cargo test -p ezsdr-radio-uhd --lib brain::`);
- the ignored walk sweeps (`*_walks`, `EZSDR_SEEDS=a-b`, release build), at 10,000 seeds per case (§7: "seed 10,000 個");
- the ignored micro exhaustive searches (`*_micro_full`, release, each under its recorded budget: "一つの場合は 1 コアで 10 分以内"; a search cut off by its budget counts as **not found** for that test, not as a failure of the error).

Read `plan/phase8/reviews/45e1-test-plan.md` and `45-experiment.md` §3–§5 for the cases, oracles (I1–I16, P1–P5) and witnesses.

## Part 1 — translate the 20 errors (report and stop)
1. For each holdout error, write the **smallest** source change in `brain/` (`core.rs`, `cell.rs`, `rules.rs`, `world.rs` executor programs, or `model.rs` where the error is about the device semantics) that makes the code behave the way the error describes. Write it as a row: `id`, `holdout` (the error's id and one line of its text), `file`, `old`, `new`. Use a Rust token anchor, matching exactly once (`python3 plan/maintenance/tools/anchor.py` or the logic `rules.py` uses).
2. Forbidden:
   - a change to an oracle, an invariant check, a witness list, a test assertion or a case definition;
   - a change in `threads.rs`, `shared.rs`, `force.rs`, `observed.rs` or `speed/` (the DST must find the error, not the thread tests);
   - more than one behavioural error per row.
3. If an error has no faithful translation (it names something the code does not have, or every translation would only disable an oracle), say so with the reason. It then counts as **not found** unless the orchestrator rules otherwise. Do not drop it silently.
4. Check that each mutant builds in the copy.
5. Write the rows to `~/.cache/ezsdr-fixes/2026-11-01-n45/e3/holdout-rows.json`, and a table `e3/holdout-translation.md` with these columns: error, translation, why it is faithful, and what an equivalent alternative translation would be.

**Report Part 1 (at most 15 lines) and stop.** Sol, the author of the holdout, checks the translations before any run.

## Part 2 — run them (after the orchestrator's go)
For each row, in its own copy (or one copy, reset with fresh timestamps between rows; never interleave two copies' builds in the shared target), run the DST:
- the default brain suite;
- every walk sweep at seeds 1–10,000;
- every micro exhaustive search.

Run the heavy parts on sim02 or sim03 through `python3 plan/maintenance/tools/remote/rdev.py --host <h> -- <command>`. Check that rdev works from a copy. If it needs the main checkout, run locally instead and say so; local load is allowed.

Record per row:
- found or not found;
- the first test that failed, and the invariant or witness it names;
- for a walk, the seed and the trace length;
- wall time.

A row "found" only by a thread test or by a conformance test does not count: the criterion is the DST. Record it anyway. Write the results to `e3/holdout-results.md` and the raw logs to `e3/logs/`.

## Part 3 — the past-bug table
Use `~/.cache/ezsdr-fixes/2026-11-01-n45/e3-past-bugs-facts.md`. It gives, for each of the 10 bugs, its sequence, its fix and how it maps to note-45 concepts. Its E1 coverage was read at commit e0ca15c; recheck that against the current tree.

For each bug:
1. Decide "cannot happen in this form", "found by the DST" or "not solved".
2. For "found by the DST", inject the bug as a row, the way Part 1 does: the smallest change that re-creates its sequence in the brain. Run the DST as in Part 2, and name the test that finds it.
3. For "cannot happen", give the structural reason with `path:line`: the rule or the absence of the concept.
4. For "not solved", say what the experiment does not model (for example uhd-control's record fold behind L1 M3), and whether the full design (M1) would have to.

Write the table to `e3/past-bugs.md`.

## Report (Part 2 and 3, at most 25 lines)
- The holdout score (found out of 20), with the not-found rows and why the DST misses each one: a missing case, a missing oracle, budget, or no faithful translation.
- The past-bug table in short.
- Wall times and hosts.
- Anything in this brief you think is wrong.
