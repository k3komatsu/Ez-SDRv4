# Brief for Sol: code review of note-45 experiment stage E2a-1, read-only

You are the independent reviewer (AGENTS.md §9 step 7) of the repository at ~/local_work/Ez-SDRv4. You are read-only: edit nothing, format nothing, run no cargo command that writes into the tree, and change no git state. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.**

## What changed (uncommitted; `git diff HEAD` plus untracked files)
- New files in `crates/ezsdr-radio-uhd/src/brain/`: `threads.rs`, `model_device.rs`, `force.rs`, `observed.rs`, `shared.rs`, `thread_tests.rs`.
- Changed: `rules.rs` (14 pure decisions moved out of `world.rs`), `world.rs`, `cell.rs`, `model.rs`, `oracle.rs`, `cases.rs`, `mod.rs`.
- Mutation rows: E2-01…E2-35 appended to `plan/maintenance/tools/mutations.json`, and 14 E1 rows retargeted to the moved code (E1-76, 77, 92, 96, 97, 98, 108, 111, 142, 143, 150, 151, 159, 171).

## Inputs
- Brief: `~/.cache/ezsdr-fixes/2026-11-01-n45/brief-e2a.md`.
- Test plan, with the amendments for your three reviews in §13–§15 and the results in §16: `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan-e2a.md`.
- Your plan reviews: `sol-e2a-plan.md`, `sol-e2a-plan2.md`, `sol-e2a-plan3.md`, and the dispositions in `dispositions-sol-e2a-plan.md`.
- Design: note 45, `plan/phase8/45-uhd-decision-core.md` §4 and §7.
- Plan: `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md` §1, §3, §5 and the §10 last row.

E2a-1 covers sequences S1–S5 and S10. S6–S9 come in E2a-2.

## Review
1. **Your own scenarios first.** Write them from the code, not from the plan: where the threaded wrapper can be wrong while S1's forced replay and the `Observed` comparison still pass. Think of protocol steps the forcing hook does not cover, actions taken between hook points, a free run behaving differently from a forced one, cell locking (scratch-swap, lock order, unwind restoration, I10's counter), `F` and `G` handling, shutdown and joins, and timers.
2. **Fidelity.**
   - Do the threads reuse `Brain::step`, `cell.rs` and `rules.rs` for every decision?
   - Is any decision left duplicated between `world.rs` and `threads.rs`?
   - Did the move of logic out of `world.rs` change E1's behaviour? Compare against `git show HEAD:<path>`.
   - Does every retargeted E1 row still mutate the same program?
3. **Plan.** Is each of S1–S5 and S10, and the F1–F3 corrections from your third review, implemented as the plan says? Does each assertion observe what it claims? Would its row break it?
4. **The Q1 finding** (plan §16). Production records a value's acceptance from the time read after `apply` returns. The DST world records the device's effective instant. 252 of 3,900 replayed walks diverge, classified as 246 "value" and 6 "visibility".
   - Is the classification sound?
   - Is the finding real for production (`provider/control.rs:882`, `provider/core.rs:240,307`)? Which observable outcome would differ for a user, and is the production rule's direction conservative or unsafe?
   - Give a minimal sequence.
5. **Maintainability** (AGENTS.md §9). The step came to about 1,880 lines of code and 835 of tests, against an estimate of 1,900 and a ceiling of 2,000. Name concrete duplication, scattered state, test setup that could be shared without hiding sequences, leftovers and oversized rows. Do not give line-count targets.

## Output
- Verdict: PASS or REVISE.
- Findings P1/P2/P3 with `path:line`. A P1 comes with a minimal sequence and its observable wrong outcome.
- Your answer on Q1.
- At most 120 lines.
- End with "Not checked:".
