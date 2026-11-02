# Brief: E1 fix round 1 — Sol's code-review findings

You are the implementer (AGENTS.md §9 step 8). Repository ~/local_work/Ez-SDRv4, branch main, edit in place (you are the only editor of the tree now). Read AGENTS.md first. Do not cd into v3/. Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

## Input
- Sol's review: `~/.cache/ezsdr-fixes/2026-11-01-n45/sol-e1-code.md`. It has six P1 and two P2 findings, each with a counterexample and `path:line`.
- The orchestrator's dispositions, all adopted, with decisions where Sol left a choice: `~/.cache/ezsdr-fixes/2026-11-01-n45/dispositions-sol-e1.md`. **Implement exactly these decisions.**
- The plan, with every earlier decision in its §10 rows (drafts 5–9): `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md`.
- The code is uncommitted in `crates/ezsdr-radio-uhd/src/brain/` (core.rs, cell.rs, rules.rs, model.rs, dst.rs, toy.rs, world.rs, oracle.rs, cases.rs), with rows in `plan/maintenance/tools/mutations.json`.

## How
- For each P1, first turn Sol's counterexample into a failing test (pure core, cell or model test, or a small world or scripted trace), then fix the cause. Do the same for P2 7 and 8 where practical.
- Add or retarget one row per behaviour change, from E1-131 on. Retarget or remove the rows that enforce the withdrawn behaviour (E1-107, E1-118, and the epilogue-end timer row), with the reason.
- Keep every decision in one owner: no second copy of a rule, and lifecycle facts stay in the cell.
- Remove the hand-written DFS loops in world.rs tests (note n1) in favour of `dst::explore` / `assert_bounded` with focused checks.

## After the fixes
- Run the default brain suite in debug, with its wall time, and clippy `--all-targets -- -D warnings`.
- In release, run each micro `_full` variant once (2 × 10^6 states, 180 s), one rdev command at a time, and record each verdict, its states and its missing witnesses.
- Run every case's walk sweep with 200 seeds, and the seeded configuration at horizon 96 with 200 seeds, with an env seed range. Record failures and unreached witnesses. A sweep that misses a declared witness now fails, per finding 6.
- If anything new fails, shrink and classify it: (a) world or harness, (b) core, (c) plan gap. Fix (a) in a few lines with a test and a row. Report (b) and (c) with an ignored counterexample test. Do not change core decisions beyond the dispositions.

## Build and test (owner: never put build or test load on this PC)
- Every cargo command runs through `python3 plan/maintenance/tools/remote/rdev.py -- …` on sim02, one at a time, in the foreground, each under 10 minutes. Never run cargo build, test or clippy locally. The only local cargo command is `cargo fmt --all`.
- Then run `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `python3 plan/maintenance/tools/rules.py --base HEAD`. Append rows to `mutations.json` without re-serializing it. Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout.

## Self-review
Waived. Sol re-reviews the P1 fixes next.

## Report (at most 30 lines)
- Per finding: the test that failed first, the fix (`path:line`), and its row(s).
- The rows retargeted or removed, with their reasons.
- The micro `_full` table, and the sweep results with any unreached witnesses.
- New counterexamples with their classification.
- The default suite's count and wall time, and the clippy result.
