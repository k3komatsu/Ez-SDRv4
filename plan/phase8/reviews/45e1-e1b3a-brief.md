# Brief: stage E1b part 3a of the design-note-45 experiment — case scripts and the measurement

You are the implementer (AGENTS.md §9 step 4). Repository ~/local_work/Ez-SDRv4, branch main, edit in place (you are the only editor of the tree now). Read AGENTS.md first. Do not cd into v3/. Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

## Contract
- Experiment plan: `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md`, §3.3 (search, measure first, budget overrun reporting) and §4 (cases). Draft 5 is at the end of the §10 table: it records the decisions taken during implementation, including the epilogue-end rule and P2 after a panic.
- Test plan: `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan.md`, section D (common parameters, cases 1–9 with their scripts, reduced variants, witnesses, and the seeded configuration).
- Already done and uncommitted in `crates/ezsdr-radio-uhd/src/brain/`: E1a (`core.rs`, `cell.rs`, `rules.rs`, `model.rs`), the explorer (`dst.rs`, `toy.rs`), the World (`world.rs`) and the oracles (`oracle.rs`). Read them first. `oracle.rs`'s `oracle_happy_path_passes` and `oracle_happy_path_full` show how a world is built and explored.

## What this part is for
The plan says: measure first. Before any case runs at full horizon, each case's reduced variant runs at horizons 8, 16 and 24, and its states, transitions and time are recorded and extrapolated.

A data point already exists. The tiny happy-path world (one stream, one value, a Stop, horizon 1, epilogue 1) has 166,485 states. One more tick of epilogue multiplies that by about 1.5. A larger version did not finish within 2,000,000 states. So expect overruns, and report them. Never shrink a case, narrow a window, merge events, drop a witness, add a reduction, or call a partial exploration a pass (45-experiment §3.3).

## Scope
- New file `brain/cases.rs`. One builder per case returns a `World` and a `dst::Config` (name, budget, declared witnesses) for a given variant and horizon. Cover cases 1, 2, 3, 4, 5, 6 (with `k`), 7a, 7b, 7c, 8 and 9, each exactly as test plan section D scripts it, and the seeded configuration (a builder taking the seed's choices).
  - Case 8 uses `Request::Close` and then `Request::Open` as its lifecycle admissions; the plan's "channel set change" inside StartNext is dropped by the orchestrator.
  - The rate read is not modelled. That is a declared deviation.
- **Witnesses:** add to the monitor the witness bits each case declares (W1a–W9c, the W-names of section D), recorded at the event where each happens. They must be finite and part of the key. `World::witnesses` returns them by their section-D names. If a witness cannot be observed with the current events, say which, and why.
- **Move** (cases 9 and seeded): an Ok `Move` result moves the channel from the source cell's channel set to the destination's, as one cell section in `S.result`. The values already reserved stay with the chain they were applied to; per case 9, a partial result is carried as `left` by the source chain. Nothing else about Move changes. Add one pure test, and add row E1-97 (the move result not changing the channel sets), killed by that test.
- **Measurement:** an `#[ignore]` test, `measure_reduced_cases`, with one test-name filter per case so that each case can be run on its own. For each case it explores the reduced variant at horizon 8, then 16, then 24. Each run has a budget of 2,000,000 states and a wall limit of 180 s. Add the wall limit to `Config` if the explorer lacks one. It must report `BudgetExceeded` with the usual report, never `Pass`. Once a horizon overruns, the larger ones are not run; they are recorded as "not run (smaller horizon overran)".
  - For each run it prints one line: case, horizon, verdict, states, transitions, seconds, peak RSS (`VmHWM` from `/proc/self/status`), and the witnesses reached and missing.
  - For case 6, run k = the first brain step and k = the step that processes Stop.
  - For the seeded configuration, run 50 seeds of `walk` at horizon 32 and print the failures and the witness union.
- **How to run the measurement:** run each case's measurement through rdev, one case per command, each command under 10 minutes: `python3 plan/maintenance/tools/remote/rdev.py -- cargo test --release -p ezsdr-radio-uhd --lib brain::cases::measure_<case> -- --ignored --nocapture --test-threads 1`.
  - `--release` is allowed for the measurement only. Say so in the report.
  - Collect every printed line into the report table.
- **Not in this part:** the case tests (`caseN_*_reduced` / `_exhaustive`), the section-C tests that check "every visited state of case N", and the remaining rows (E1-22, 23, 41, 44, 47–49, 52–57, 64). They follow in part 3b, once the orchestrator and the owner have seen the measurement.
- **Bugs:** if a run ends in `Violation`, `Cycle` or `Incomplete` rather than `BudgetExceeded` or `Pass`, it may be a real bug. Shrink the trace, put it in the report, and do not fix it unless the fix is a few lines and plainly the plan's behaviour. Never weaken an oracle.

## Build and test (owner: never put build or test load on this PC)
- Every cargo command runs through `rdev.py` on sim02, one at a time, in the foreground. Before reporting, run the default brain suite (`cargo test -p ezsdr-radio-uhd --lib brain`, debug) and clippy (`--all-targets -- -D warnings`). Never run cargo build, test or clippy locally. The only local cargo command is `cargo fmt --all`. After writing, run `git ls-files -m -o --exclude-standard | xargs chmod go+r`, then `python3 plan/maintenance/tools/rules.py --base HEAD`.
- Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout.

## Self-review
Waived for this part. The measurement is data and the orchestrator reviews it. Sol reviews all of E1 later.

## Report (return this, at most 40 lines)
- Files and line counts.
- The measurement table, one line per run.
- The growth per horizon step for each case, and an extrapolation to the full horizon (states and time on one sim02 core). Say which cases fit the plan's budget of 10 minutes and 5 × 10^6 states, and which do not.
- Every witness not reached, with the reason.
- Every `Violation`, `Cycle` or `Incomplete`, with its shrunk trace.
- The default suite's count and time, and the clippy result.
- Every deviation from the plan.
