# Brief: stage E1b part 1 of the design-note-45 experiment — the generic explorer and the toy models

You are the implementer (AGENTS.md §9 step 4). Repository ~/local_work/Ez-SDRv4, branch main, edit in place (you are the only editor of the tree now). Read AGENTS.md first. Do not cd into v3/. Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

## Contract
- Experiment plan (draft 4): `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md`, §3.3 (search) is this part.
- Test plan (approved by Sol): `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan.md`, section C. This part implements only the tests of section C that run on toy models (list below). The tests on `World` and the cases (the rest of C, D, E) are parts 2 and 3, not now.
- Stage E1a is done and uncommitted in `crates/ezsdr-radio-uhd/src/brain/` (`core.rs`, `cell.rs`, `rules.rs`, `model.rs`, `mod.rs`). Do not change E1a code in this part. If something there blocks you, stop and report it.

## Scope
- `brain/dst.rs`: the generic explorer. It is generic over a small model trait, so the toys now and `World` in part 2 use the same code. The trait has state, an ordered exact key (no hash, `BTreeMap`/`BTreeSet` only, no `HashMap` anywhere in state or search), enabled events, apply, a safety check, a terminal (progress) check and the witnesses a state records. Diagnostic counters and traces stay outside the key.
  - `explore`: DFS over enabled events with a visited set keyed by the exact key. The safety check runs on every transition's resulting state **before** the visited-set lookup, so a violating edge into an already visited state is caught (E1-61). Each distinct state is expanded once, and each of its outgoing edges is traversed exactly once (E1-30). A revisit of a state on the DFS stack is `Cycle` with its trace (E1-31), and a revisit of a state off the stack is reconvergence, not a cycle. The progress check runs on every terminal state (no enabled event). A state budget overrun is `BudgetExceeded` and never `Pass` (E1-32). Its report names the configuration, states, transitions, elapsed time, memory, reached and missing witnesses and the unexplored frontier. A declared witness that no path reaches gives `Vacuous` naming it. The verdict carries the counts: states, transitions, elapsed time.
  - `walk(seed)`: a seeded random walk over the same events. The RNG is the SplitMix64 already in `crates/ezsdr-radio/tests/generator/mod.rs:230`, reached through `provider::generator`. Make that `#[cfg(test)]` module `pub(crate)` in `provider/mod.rs`; this is the only change outside `brain/`, it is test-only, and you must not copy the RNG. The RNG must not be part of the model state.
  - `replay(trace)` is deterministic. `shrink(trace)` returns a minimal failing trace (greedy removal until no single event can be dropped) that still fails.
  - `assert_complete` (the case wrapper) panics on any verdict other than `Pass`, printing the report.
  - Leave the hook for `Incomplete` in the verdict type (an actor still waiting at the end of the epilogue). Its test needs `World`, which is part 2.
- `brain/toy.rs`: toy-A (lost update: two actors read then write a counter, final must be 2), toy-B (two actors, opposite lock order: P4 deadlock), toy-C (self-loop), toy-D (diamond), toy-E (small acyclic graph with known n states and m edges), toy-F (a diagnostic counter changes on every event while the protocol state repeats). None of them uses the core.
- Tests of this part (section C, names as planned): `harness_dfs_expands_each_edge_once`, `harness_reconvergence_is_not_a_cycle`, `harness_cycle_is_reported`, `harness_diagnostic_counter_cycle_is_reported`, `harness_budget_overrun_is_failure`, `harness_replay_is_deterministic`, `harness_walk_same_seed_same_trace`, `harness_shrink_reaches_one_minimal_failure`, `harness_vacuous_case_fails_on_missing_witness`, `harness_broken_toy_is_caught`, `harness_violation_into_visited_state_is_caught`. They are pure: no threads, no clock reads in the model, no sleep. Elapsed time is measured only in the verdict, never in the model.
- Mutation rows of section F for this part: E1-30, E1-31, E1-32, E1-61. Append them to `plan/maintenance/tools/mutations.json` after the existing E1 rows, in the same format. Each Rust `old` anchor is a token sequence that must match exactly once (`python3 plan/maintenance/tools/rules.py --base HEAD` checks it). Each row must break its named test. Reason this out; do not run mutations.
- Size checkpoint: about 600 lines including tests. If you go well past it, stop at a coherent point with the tree green and report.
- Keep it small: no feature the listed tests and rows do not need. Long lines are fine (`rustfmt.toml`, max_width 200).

## How to build and test (owner: never put build or test load on this PC)
- Every cargo build, test or clippy runs on sim02 through `python3 plan/maintenance/tools/remote/rdev.py -- cargo …` from the repository root, one at a time, in the foreground. Examples: `… -- cargo test -p ezsdr-radio-uhd --lib brain` and `… -- cargo clippy -p ezsdr-radio-uhd --all-targets -- -D warnings`. Never run cargo build, test or clippy locally.
- The only local cargo command is `cargo fmt --all`, before reporting. After writing files, run `git ls-files -m -o --exclude-standard | xargs chmod go+r`.
- Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout. Test temp files, if any, go on /tmp.

## Self-review
Waived for this part. Part 1 is generic search machinery, its tests are toy-level, and the orchestrator reviews it before part 2 builds on it. Sol reviews all of E1 at the end.

## Report (return this, at most 25 lines)
Files and line counts (body / tests); the exact rdev commands run and their counts; the clippy result; the rows added and the test each breaks; the trait as you defined it (signatures only); every deviation from the plan and every place you found it contradictory.
