# Brief: note-45 experiment E2a-2 — FakeDevice smoke, native-lock contention, conformance (S6–S9)

You are the implementer (AGENTS.md §9). Repository `~/local_work/Ez-SDRv4`, branch `main`. Edit in place: you are the only editor of the tree. Read AGENTS.md first. Do not cd into `v3/`. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`** and do not search for it.

## Where things are
- E2a-1 is committed: the thread runtime, `ModelDevice`, trace forcing, `Observed`, S1–S5 and S10, under `crates/ezsdr-radio-uhd/src/brain/`. The files are `threads.rs`, `shared.rs`, `model_device.rs`, `force.rs`, `observed.rs`, `thread_tests.rs`, plus `rules.rs`, `cell.rs`, `world.rs`, `model.rs` and `core.rs`.
- The original brief is `~/.cache/ezsdr-fixes/2026-11-01-n45/brief-e2a.md`. Items 6 and 7 and the S6–S9 rows are this step.
- **The test plan is binding:** `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan-e2a.md`. Read §7 and §8, and then the amendments, which override them: §13 (Sol plan review), §14 (re-review N1–N4, the concrete conformance scripts a–h with times, channels and expected results, `ConsumeProbe`), §15 (F1–F3 corrections to S6b and the conformance schedules) and §16 (E2a-1 results).
- Sol's plan reviews `sol-e2a-plan*.md` and the dispositions `dispositions-sol-e2a-plan.md`, for the reasons behind the amendments.
- E2a-1's code review and its dispositions, `sol-e2a1-*.md` and `dispositions-sol-e2a1-code.md`, for the design decisions now in the code:
  - the guard routine;
  - the atomic batch abandonment;
  - publish compute outside the cell and recheck inside;
  - the `now` source.

  Do not undo any of them.

## Do
1. **S6:** the FakeDevice happy-path smoke on the real clock. It starts at a future origin, proves both streams produced samples before the value is released, and runs to Shutdown. The order of calls and the join bound are as in the plan.
2. **S6b:** the cleanup path, as §15 specifies it:
   - the initial time, progress and buffering;
   - the cut you book;
   - the Restore enqueued explicitly;
   - the calls `Stop, StartNow, Stop, Apply(restore)` in order;
   - `Reported(Left)`, plus the leftover consumption observed separately.
3. **S7:** a timed DDC value on FakeDevice. At ratio 1, the application tick and the evidence tick both equal `e`. Admission and acceptance timing are observed separately.
4. **S8:** native-lock contention on `ModelDevice`. S waits in `apply` holding `L` with a full queue; the drain deadline arrives; R's stop waits for `L`. Assert the actual acquire/release order:
   - I16 at the deadline;
   - P1: the stop is issued and waits for `L`;
   - pinned, then `left`;
   - all joins once `L` frees.

   Add the clock-contention trace (`ClockLock`, `Read`, `Unlock`, `Send`), and keep `s8b`'s timer-while-K-waits. Some of this may already exist from E2a-1 (`s8a`, `s8b`); extend it, do not duplicate it.
5. **S9:** the conformance suite, scripts a–h, as §14 and §15 give them. It runs on FakeDevice and on `ModelDevice` through the `Device` trait and compares against the stated expected results.
   - **First, check whether FakeDevice's manual clock can drive the scripts deterministically** (plan Q7). If it cannot, say how they are sequenced instead. Verdicts must not depend on wall time.
   - A hand-derived expectation that a run contradicts: do not edit it silently. Say which side is wrong (the model, FakeDevice, or the derivation) and give the evidence.
   - Fix a model fault with a test and a row. A remaining difference is waived only as §14 allows, with the affected E1 cases and the reason their verdicts stand.
   - Script (g) is expected to show the Q1 finding. Report it and classify it; never waive it.
   - Do not change FakeDevice.
6. Update `test-plan-e2a.md` §17, "E2a-2 results": each test, the conformance table (script, FakeDevice, `ModelDevice`, expected, verdict or listed difference), and the differences with their reasons.

## Checks and rows
- Run the brain suite and record its time.
- Run each timing-dependent test (the FakeDevice real-clock smoke, S8) 20 times.
- Run `cargo fmt --all`, and `cargo clippy -p ezsdr-radio-uhd --all-targets -- -D warnings` clean.
- Mutation rows continue after the last E2 row in `plan/maintenance/tools/mutations.json`, as the plan lists them for S6–S9 (renumber from the next free id). Append them as text, without re-serializing the file. Then run `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `python3 plan/maintenance/tools/rules.py --base HEAD` (0 problems).
- Never run `mutate.py` or `remote_mutate.py`. If you check rows in an out-of-tree copy, say so; the orchestrator runs them remotely.
- **Self-review: one round.** `opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free --agent plan --auto "<checklist>" > /tmp/e2a2-selfreview.log 2>&1; echo rc=$? >> /tmp/e2a2-selfreview.log`, in the background. Read the log as soon as `rc=` appears (check every minute). Use AGENTS.md §9 step 5's checklist:
  - every sequence has its test, the test observes what the plan says, and the row breaks it;
  - scope against the plan;
  - every "every/only/all" claim searched;
  - maintainability: duplication with E2a-1's tests and fixtures, setup reused without hiding sequences, leftovers.

  Fix each finding, or reject it with a reason.
- Never git commit, push, stash, reset or checkout.

## Report (at most 25 lines)
- S6, S6b, S7 and S8, each with its test and pass/fail.
- The conformance table in short, and the differences with their classification.
- The manual-clock answer.
- The Q1 result from script (g).
- The 20× results.
- Rows added.
- The self-review findings and their dispositions.
- Suite count and time, and clippy.
- Size in lines (code, tests).
- Anything in this brief or the plan you think is wrong.
