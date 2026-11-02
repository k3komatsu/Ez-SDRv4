# Brief: stage E1b part 3d of the design-note-45 experiment — the "busy stream" decide rule and case 9's refusal

You are the implementer (AGENTS.md §9 step 4). Repository ~/local_work/Ez-SDRv4, branch main, edit in place (you are the only editor of the tree now). Read AGENTS.md first. Do not cd into v3/. Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

## Background (read before coding)
- The experiment code is uncommitted in `crates/ezsdr-radio-uhd/src/brain/`: `core.rs` (the brain and its `decide`), `cell.rs`, `world.rs` (the simulated world), `oracle.rs`, `cases.rs`, `dst.rs`.
- The plan is `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md`. The §10 rows for drafts 5–7 hold the decisions so far. Draft 7 is the key one: the brain decided from cell state while work it had already handed out would change that state later. The fix was one rule per executor kind: lifecycle decisions wait while a value is `Queued` on the channel or chain (S), and while an R command is `outstanding` on the stream (R).
- Two new counterexamples show the same mechanism a fourth time, through work the brain **itself** has pending, a booked or draining stop:
  - case 8 micro (I9): `case8_micro_close_overtakes_booked_stop_counterexample`. A Stop books a drain timer. A Close request waits only for a committed value, and is sent once that value's `Done` arrives. Then the drain timer fires and sends the Stop behind the Close, and R calls Stop on a closed handle.
  - case 7b micro (I13): `case7b_micro_restore_during_stop_counterexample`. A Restore is decided while the cell is `Stopping`. The flush's final stop then fails (`Uncertain`), but the Restore is already on its way to S.
  - Production gets this order from one owner: the RX thread runs stop, then flush, then restore in sequence, and decides the restore only after the flush (provider/rx.rs:300-335).
- Both are `#[ignore]` tests today.

## Rule to implement (orchestrator's decision; it amends the decide conditions)
A stream is **busy** while it has an outstanding R command, while its drain is `Booked` or `Draining` (a stop is decided but not yet sent), or while it is `Stopping` with its stop not yet finished. Write it once, as a cell method, for example `Cell::busy()`. Make the method name and its doc comment say exactly that.
- StartNext, Open, Close and Flush for a stream wait while that stream is busy.
- A Restore of a channel waits while any stream whose channel set contains that channel is busy, in addition to its existing conditions (pins and queued values, debt and left, `Uncertain`/`Lost` as `Never`).
- A Move waits while its source stream is busy.
- Reuse the existing `outstanding` and `drain` state. Add no new field unless "Stopping with its stop not yet finished" cannot be read from the existing state. If you add one, say why.
- Check every existing caller of the pieces this rule subsumes. Remove any guard that becomes redundant (for example a separate `drain` or `outstanding` check in one `decide` arm), and name each one.

Turn the two counterexample tests into default pure core tests (or small-world tests) that fail before the fix and pass after. Add rows E1-123 (the busy check removed from Close), E1-124 (the busy check removed from Restore) and E1-125 (`busy` ignoring the drain), each killed by its test.

## Case 9's refusal
W9a.refused needs the channel-0 DSP FIFO to be full when v's settings call runs, and the case has one value, so no FIFO size reaches both refused and ok/partial. Do not add a released value. Give case 9's script (full, reduced and micro) an **initial DSP FIFO occupant** on channel 0, an earlier timed write already queued in the device. This is like case 5's radio-queue occupant, so it is initial device state, not a new Env input. Give it an instant inside the horizon, so it drains at its sample and frees the slot. Set `fifo_cap` to 1. Then refused is reachable while the occupant is queued, and ok/partial after it drains. W9b ("the move issued after the pin cleared"): find why it is missing in the micro variant and fix the script if the cause is a script error. Otherwise report it.

## After the changes
- Run the default brain suite in debug, and clippy `--all-targets -- -D warnings`.
- In release, run each micro variant once: cases 1, 2, 3, 4, 5, 6 (both k), 7a, 7b, 7c, 8 and 9. Use the existing caps (2 × 10^6 states, 180 s). Run them one rdev command at a time, each under 10 minutes. Record each verdict, its states and its missing witnesses.
- Run `seeded_config_reduced`, and run the seeded walks at horizon 96 for seeds 1..200 with the existing sweep test and an env seed range. Record the failures.
- If a micro run or a seed shows a new `Violation`, `Cycle` or `Incomplete`, shrink it and classify it: (a) world or harness, (b) core, (c) plan gap. Fix (a) in a few lines with a test and a row. For (b) and (c), write the counterexample as an ignored test and report it. Do not change core decisions beyond the busy rule above.

## Build and test (owner: never put build or test load on this PC)
- Every cargo command runs through `python3 plan/maintenance/tools/remote/rdev.py -- …` on sim02, one at a time, in the foreground, each under 10 minutes. Never run cargo build, test or clippy locally. The only local cargo command is `cargo fmt --all`.
- After writing, run `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `python3 plan/maintenance/tools/rules.py --base HEAD`. Append rows to `mutations.json` without re-serializing it. Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout.

## Self-review
Waived. The orchestrator reviews, and Sol reviews E1 as a whole next.

## Report (at most 30 lines)
- The busy rule: the method, its callers, and the guards removed.
- The tests and rows.
- Case 9: the occupant, and W9b's cause.
- One line per micro run: verdict, states and missing witnesses.
- The seeded results.
- New counterexamples with their classification.
- The default suite's count and wall time, and the clippy result.
