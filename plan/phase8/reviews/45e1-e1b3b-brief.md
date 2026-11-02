# Brief: stage E1b part 3b of the design-note-45 experiment — micro cases, case-level harness tests, walks

You are the implementer (AGENTS.md §9 step 4). Repository ~/local_work/Ez-SDRv4, branch main, edit in place (you are the only editor of the tree now). Read AGENTS.md first. Do not cd into v3/. Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

## Contract
- Experiment plan: `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md`. Read §3.3, §4 and the §10 rows for drafts 5, 6 and 7, and the owner's search decision. That decision says: each case keeps an exhaustive **micro variant**, with instants moved early and the horizon and epilogue shortened until the search completes within the budget, and every declared witness still reached. Full-horizon cases and the seeded configuration are checked by **seeded random walks**. A walk sweep is evidence of bugs found, never a claim of completeness.
- Test plan: `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan.md`, sections C (harness tests), D (cases, witnesses) and F (rows).
- Current state, uncommitted, in `crates/ezsdr-radio-uhd/src/brain/`: `core.rs`, `cell.rs`, `rules.rs`, `model.rs`, `dst.rs`, `toy.rs`, `world.rs`, `oracle.rs`, `cases.rs`. Read them first. The last replay table, every case at reduced horizon 8, is in `/tmp/claude-1001/-home-komatsu-local-work-Ez-SDRv4/8aebdd6c-e346-4ee3-8da4-65b3b7a7fb57/scratchpad/replay-table.txt`. No case violates, and none completes within 2 × 10^6 states.

## Scope
1. **Micro variants.** For each case (1, 2, 3, 4, 5, 6 with its two k, 7a, 7b, 7c, 8, 9) add a micro builder in `cases.rs` and a default test `caseN_<name>_micro`. The test runs `explore` to `Pass` with every witness the case declares, through `assert_complete`.
   - Shrink only the time scale. Move the instants, windows, cuts and origins early, shorten the horizon and epilogue, and lower the recv timeout and drain window if needed, keeping the order relations each witness needs (e.g. e < cut, cut below e for a cancel).
   - Never drop an Env input, an outcome branch or a witness, and never merge events.
   - Target: each micro test under about 20 s in a debug build on sim02. Its killer rows run it under mutation, so it must stay fast.
   - If a case cannot reach all its witnesses within about 20 s, give it the smallest micro variant that does, mark that test `#[ignore]`, and run it in release. Report its counts.
   - If a witness cannot be reached at any size you try, report it with the reason. Never weaken the declaration silently.
   - Name each micro variant's parameters in a doc comment.
2. **Case-level harness tests of section C** not yet present: `harness_no_effect_before_take`, `harness_every_event_changes_state`, `harness_time_only_advances_on_tick`, `harness_tick_cannot_pass_mandatory_input`, `harness_clock_read_holds_native_lock`, `harness_result_precedes_send`, `harness_epilogue_covers_deadlines`, `harness_loss_publish_releases_l_first`, `harness_lifecycle_inline_loss_publish_no_recursion`. Add `harness_timer_never_early` if it is missing.
   - Each checks its property in every visited state, or at every transition, of the micro variant of the case the plan names.
   - Use a small wrapper `Model` around `World` that adds the extra check as a `transition` or `safety` check. Do not put test-only checks into `World` itself.
3. **Rows.** Add the planned rows not yet present: E1-22, 23, 41, 44, 47, 48, 49, 52, 53, 54, 55, 56, 57, 64. Each targets the code it names (`world.rs` or `core.rs`, wherever that behaviour now lives; `dst.rs` has no R loop) and is killed by its named test, now the micro variant of that case or the section-C test.
   - Rust anchors are token sequences matching exactly once.
   - Append to `mutations.json` without re-serializing it, so the diff shows only appended or edited E1 rows.
   - Run `python3 plan/maintenance/tools/rules.py --base HEAD`.
   - If a planned row's behaviour no longer exists, because the design changed it, say so instead of inventing a target.
4. **Walks.**
   - (a) `seeded_config_reduced`, default: 50 seeds at horizon 32, asserting no failure and reporting the witness union.
   - (b) One `#[ignore]` walk sweep test for each case at its full horizon, and one for the seeded configuration at horizon 96. The seed range and the case come from environment variables (for example `EZSDR_SEEDS=1-2000`). Each failure prints its seed and its shrunk trace. The test fails if any seed fails, and prints how many seeds ran, the wall time and the witness union.
   - Do not run the large sweeps yourself. The orchestrator runs them on sim02 and sim03. Run each sweep test once, with 20 seeds, to show that it works.
5. **Test names.** The plan's `_reduced` and `_exhaustive` names become `_micro` (default, exhaustive) and `_walks` (ignored, sweep). Say so as a deviation. Remove any measurement test that this part makes redundant, but keep `measure_reduced_cases`: its numbers are the evidence for the owner's decision.

## Build and test (owner: never put build or test load on this PC)
- Every cargo command runs through `python3 plan/maintenance/tools/remote/rdev.py -- …` on sim02, one at a time, in the foreground, each under 10 minutes. Release builds are allowed for the ignored micro runs and the sweep checks only.
- Before reporting, run the default brain suite in debug and give its wall time, plus clippy `--all-targets -- -D warnings`. Never run cargo build, test or clippy locally. The only local cargo command is `cargo fmt --all`. Then run `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `rules.py --base HEAD`.
- Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout.

## Self-review (one round, before reporting)
Run from the repository root, in the foreground: `timeout 2400 opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free --agent plan --auto "<checklist>" > /tmp/e1b3b-review.txt 2>&1; echo rc=$? >> /tmp/e1b3b-review.txt`.
- If the file is empty after 3 minutes, kill the process by PID and relaunch once. If the free model refuses for quota, use `opencode-go/deepseek-v4.1-flash`.
- **The command returns when the review ends. Read `/tmp/e1b3b-review.txt` immediately after it returns. Never wait for a notification, and never start the review in the background.**

Checklist to pass: read 45-experiment §4 and §10 (drafts 5–7 and the owner's decision), test plan sections C, D and F, `cases.rs` and the diff, then report:
1. Each micro variant keeps every Env input, outcome branch and witness of its case, and the order relations its witnesses need. Name any variant that changed more than the time scale.
2. Each section-C test checks its property in every visited state or transition of the named case.
3. Each new row targets live behaviour and breaks its named test.
4. The walk sweeps report failures with seeds and shrunk traces, and never claim completeness.

Read-only. Fix each finding or reject it with its reason.

## Report (return this, at most 30 lines)
- One line per micro test: parameters, states, seconds, debug or release.
- The section-C tests added.
- The rows added, and the planned rows not added with the reason.
- The walk tests and their 20-seed check.
- The default suite's count and wall time, and the clippy result.
- The self-review findings and their disposition.
- The deviations.
