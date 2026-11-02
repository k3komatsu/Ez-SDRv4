# Brief: stage E1b part 3c of the design-note-45 experiment — an exact, canonical search key, then the open fixes

You are the implementer (AGENTS.md §9 step 4). Repository ~/local_work/Ez-SDRv4, branch main, edit in place (you are the only editor of the tree now). Read AGENTS.md first. Do not cd into v3/. Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

## Background
The experiment code is uncommitted in `crates/ezsdr-radio-uhd/src/brain/`. `World::key` (world.rs, `impl Model for World`) is the Debug text of nearly every field. Part 3b found that the micro case variants still exceed 2 × 10^6 states, because states that behave identically rarely merge: the key carries **history**, meaning the order in which facts were written and logs of past events, where only the facts that decide future behaviour or a later verdict should count (45-experiment §3.3: "the monitor enters the key as a finite summary ... not histories").

This part makes the key exact and canonical. It is not a reduction. Two states with equal keys must have the same future events and the same verdicts. Every item below has a soundness argument. Write it as a one-line comment at the place in `key` that implements it.

## Part A — canonical key (do this first, then measure)
Change only the **key**, not the state. Where a field is history, the key uses its canonical form; the state keeps the field as it is, so tests and the monitor still see it.
1. `Brain::dispositions`, `Brain::results` and `Brain::leftovers` are `Vec`s in write order. Code reads them only by id (first write per id wins, `dispose` / `disposition` / `consumed`). The key uses them sorted by id. Check every reader with a search and list them in the report. If any reader depends on order across ids, stop and report.
2. `Monitor::observed` holds `(stream, at, tick)`; the tick of delivery is history. The key uses `(stream, at)`. First check which oracle reads the tick; if P1 or another check needs it, say so and keep it.
3. `Dsp::log`, the device's applied log, is history: the device's future behaviour does not read it, and `sync` copies each entry into `Monitor::applied`, which is in the key. The key leaves `log` out. Check that `model.rs` never reads `log` to decide behaviour.
4. `Dsp::flows`: a closed flow whose `end ≤ t` can no longer pass a sample, because `t` only grows. The key keeps only flows still open or ending after `t`. Check the code that reads `flows`.
5. `RxExec::consumed`: the key keeps only the ids that are still in that stream's cell. A retired value is never offered again. Check `consume`.
6. `Monitor::witnesses` and the bits that only feed witnesses: the explorer collects witnesses on every transition's resulting state, so the union over paths does not need them in the key. The key leaves out `witnesses`. Keep `shortened` only if an oracle reads it, not only a witness. Check `dst.rs` `observe`.
7. Any other field you find that only records the past and is read by no behaviour and no oracle. List each one with its readers before you change it. `next_id` and `handles` stay as they are; they are identities.

Add `harness_key_merges_equivalent_histories`: two worlds reached by different interleavings of independent writes (two dispositions written in opposite orders) have equal keys. Keep `harness_key_separates_each_behaviour_field` green; it must still separate every behaviour field.

Then **measure**. In release on sim02, run the happy path `oracle_happy_path_full` and every case's micro variant once. Each run gets the 2 × 10^6-state budget and 180 s, through rdev, one command at a time, each under 10 minutes. Report states and seconds before and after, using part 3b's numbers in `/tmp/claude-1001/micro-release.txt` as "before".

## Part B — the open fixes (after Part A's measurement)
1. **The `unreachable!` at world.rs ~903** ("a timed command's value is in a cell"; cases 2, 6, 7a). A loss can dispose and retire a value while its settings command is still in S's FIFO. In that case `S.take` finds no value. It must write the terminal `Dropped`, because the value went before its call, and send `Done { Dropped }`. The brain's `done` must ignore it for dispositions, since the value already has one. Test, row.
2. **The seeded configuration's windows.** `seeded_config_reduced` seed 2 ends `Incomplete` with undelivered env slots. A slot whose window lies past the last tick can never be delivered. The builder must draw every window inside `[0, horizon]`, keeping each input's plan window clipped to the horizon. Test: a builder check over seeds 1..200 that every window fits.
3. **Micro declarations.** Each micro variant keeps the plan's branches and order relations:
   - case 7b micro enables `call_faults`, so the flush's final stop can fail (W7b2, W7b3);
   - case 9 micro sets `fifo_cap` small enough (a size parameter, like the horizon) that a refusal is reachable (W9a.refused);
   - case 5 micro keeps the occupant's due time after v's instant (full: 40 > 8);
   - case 7c micro keeps the Stop's requested instant above v1's instant (full: 6 > 4);
   - case 6 declares W6a only for the k that processes Stop, since a panic at the first step precedes any value (a plan gap; I record it).
4. **Case 8 micro, I9 violation** ("stream 0 calls Stop{epoch:1} on a closed handle"). Shrink it, print the brain section that sent the Stop and the one that sent the Close, with file:line, and classify it: (a) world or harness, (b) core, (c) plan gap. Fix (a). For (b), write the counterexample as a failing test and propose the rule. Do not change core decisions before the orchestrator answers. Then stop.
5. Then rerun the micro tests and the section-C tests that use them. Each micro test that completes in under about 20 s in debug becomes a default test. The others are `#[ignore]` and run in release, with their counts in the doc comment.

## Build and test (owner: never put build or test load on this PC)
- Every cargo command runs through `python3 plan/maintenance/tools/remote/rdev.py -- …` on sim02, one at a time, in the foreground, each under 10 minutes. Never run cargo build, test or clippy locally. The only local cargo command is `cargo fmt --all`.
- Then run `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `python3 plan/maintenance/tools/rules.py --base HEAD`. Append rows to `mutations.json` without re-serializing it. Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout.

## Self-review
Waived. The orchestrator reviews the key change line by line, and Sol reviews E1 as a whole next.

## Report (at most 30 lines)
- Part A: each canonicalization with its readers checked. The before/after table (happy path and each micro: states, seconds).
- Part B: each fix with its test and row, the case 8 classification with its trace, and the micro tests that are now default or ignored.
- The default suite's count and wall time, and clippy.
