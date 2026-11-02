# Brief: stage E1b part 2b of the design-note-45 experiment — the oracles

You are the implementer (AGENTS.md §9 step 4). Repository ~/local_work/Ez-SDRv4, branch main, edit in place (you are the only editor of the tree now). Read AGENTS.md first. Do not cd into v3/. Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

## Contract
- Experiment plan (draft 4): `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md` §5. This part implements the safety properties I1–I9 and I11–I16, the progress properties P1–P4 (P5 is the explorer's cycle check), and the three kinds of fact.
- Test plan (approved by Sol): `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan.md` section E, the oracle table. It gives each oracle's evaluation point, what it reads, the wrong behaviour it must catch, and its test names.
- Already done and uncommitted in `crates/ezsdr-radio-uhd/src/brain/`: E1a (`core.rs`, `cell.rs`, `rules.rs`, `model.rs`), part 1 (`dst.rs`, the explorer, whose `Model` has `safety` for state checks on key data only, `transition(before, event, after)` for checks of a step, `progress` at terminal states, and `incomplete`), and part 2a (`world.rs`, the World, its events and the monitor). In 2a, `World::safety`, `transition` and `progress` return `Ok(())`. This part fills them. Read these files first.

## Scope
- New file `brain/oracle.rs`, with one function per property. Each reads the World and its monitor and returns `Err` with a text that names the property (`"I4: ..."`).
- `World::safety`, `World::transition` and `World::progress` in `world.rs` call these functions. Put each property where its evaluation point says:
  - A property about the current state goes in `safety`. Examples: I5's cut below progress, I8's per-sample take count, from the device log.
  - A property about what one step did goes in `transition`. These are evaluated "at each reservation / take / disposition / command / admission / publication / ack / drain deadline": I1, I2, I3, I4 at its write, I6, I7, I9, I11, I12, I13, I14, I15, I16.
  - P1–P4 go in `progress`, at terminal states.
  - `safety` reads only data in the key. Any fact a check needs from the past goes into the monitor as a finite summary, which is in the key, never as a history. If a property needs a fact the monitor does not record yet (for example, for I1, "the actor's most recent F check saw F, and no call since"), add it to the monitor or to the executor's state. Say so in the report.
- The three kinds of fact stay apart, as §5 and section E say: device application (`applied`), qualifying receive evidence (`evidence`, published or not), and publication. A value has one disposition, a command one terminal, and a leftover consumption is a separate fact. I4's identity check compares value, channel, segment, epoch and sample.
- **Tests:** every test named in section E's last column, except the two P5 tests that already exist in part 1. Each is a crafted world, or one crafted transition, that violates exactly its property, and asserts the `Err` names it. Build the world through the World constructor and a few direct field edits; reuse the existing fixtures.
  - Add one positive test, `oracle_happy_path_passes`. A tiny world (one stream, one value released and consumed, a Stop, Shutdown) explored with all oracles on returns `Pass`. It catches an oracle that fires on legal behaviour. Keep the world small enough to explore in under a few seconds on sim02.
  - If the happy path fails because of a real bug in core, cell, rules, model or world rather than in an oracle, do not weaken the oracle. Report the minimal trace (use `shrink`), fix it only if the fix is a few lines and plainly the plan's behaviour, and name it in the report.
- **Mutation rows:** section F has no rows on `oracle.rs`. The oracles' power is shown by their negative tests, and later by the case runs. Add no rows unless you change core/cell/rules/model/world behaviour; for any such change, add a row and a killing test.
- **Not in this part:** case scripts, the C tests "in every visited state of case N", and measurement. Those are part 3.
- **Size:** checkpoint about 900 lines including tests. Well past it, stop at a coherent point with the tree green and report what remains. Never compress code or drop planned tests to fit. Where the plan is contradictory, implement its safest reading and report it.

## How to build and test (owner: never put build or test load on this PC)
- Every cargo build, test or clippy runs on sim02 through `python3 plan/maintenance/tools/remote/rdev.py -- cargo …` from the repository root, one at a time, in the foreground. Examples: `… -- cargo test -p ezsdr-radio-uhd --lib brain` and `… -- cargo clippy -p ezsdr-radio-uhd --all-targets -- -D warnings`. Never run cargo build, test or clippy locally.
- The only local cargo command is `cargo fmt --all`, before reporting. Then run `git ls-files -m -o --exclude-standard | xargs chmod go+r`.
- Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout.

## Self-review (one round, before reporting)
Run from the repository root, in the foreground: `timeout 2400 opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free --agent plan --auto "<checklist>" > /tmp/e1b2b-review.txt 2>&1; echo rc=$? >> /tmp/e1b2b-review.txt`.
- If the file is still empty after 3 minutes, kill the process by PID and relaunch once.
- If the free model refuses for quota, use `opencode-go/deepseek-v4.1-flash`.
- The run ends when `rc=` appears in the file. Read the file then; do not wait for any other notification.

The checklist to pass: read 45-experiment §5, test plan section E, `brain/oracle.rs`, `world.rs` and the diff, and report:
1. For each property, whether the code evaluates it at the plan's point and catches the wrong behaviour the table names. Name each property that is weaker, missing, or placed in `safety` while it reads data outside the key.
2. Each negative test violates exactly its property and asserts it by name.
3. Any oracle that could fire on legal behaviour (false positive), with the sequence.
4. The monitor facts added: finite, in the key, and none of them a history.

Read-only, no mutation runs. Fix each finding or reject it with its reason in your report.

## Report (return this, at most 30 lines)
- Files and line counts (body / tests).
- The exact rdev commands run and their counts, and the clippy result.
- The placement of each property (`safety`, `transition` or `progress`).
- The monitor or executor facts added.
- Any core/cell/rules/model/world fix, with its row and test.
- The self-review findings and their disposition.
- Every deviation from the plan, and every place you found it contradictory.
