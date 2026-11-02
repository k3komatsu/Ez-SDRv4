# Brief: E1 fix round 2 — Sol's re-review findings and the walk scheduler

You are the implementer (AGENTS.md §9 step 8). Repository ~/local_work/Ez-SDRv4, branch main, edit in place (you are the only editor of the tree now). Read AGENTS.md first. Do not cd into v3/. Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

## Input
- Sol's re-review: `~/.cache/ezsdr-fixes/2026-11-01-n45/sol-e1-rereview.md`. Findings 1–3 and 5–8 are closed; finding 4 is not. It adds P1-A, P1-B, P2-C and a note, each with a minimal sequence and `path:line`. **Every point is adopted.** Earlier context: `sol-e1-code.md`, `dispositions-sol-e1.md`, `brief-e1-fix1.md`, and the plan `45-experiment.md` §10 drafts 5–9, all in the same directory.
- The code is uncommitted in `crates/ezsdr-radio-uhd/src/brain/`, with rows in `plan/maintenance/tools/mutations.json` (the last is E1-166).

## Do
1. **P1-A, tail promotion timing.** Promotion of a start queued behind a stopped tail happens at the **next recv entry** after the recv that returned the tail, as FakeDevice does (`crates/ezsdr-radio-uhd/src/device.rs:1627-1640`). It must not happen on a Tick, nor at the end of the tail-returning recv. If, at that promotion, the start instant is below `now`, the recv returns `LateCommand`. Turn Sol's delayed-next-recv sequence into a regression test.
2. **P1-B, direct past starts are late** (the earlier deviation is rejected). A timed start whose instant is below `now` when the device processes it records a missed start, and the next recv returns `LateCommand`, as FakeDevice does (`device.rs:1195`). Keep timed starts and start-now apart: the flush uses start-now (`rx_start_now`) and is never late. The brain's existing LateCommand handling decides what happens next: restart when cut-free, end-now with a cut. Check every case and micro builder: a scripted initial "streaming from 0" state that starts at `T = 0` is not late. Report any case whose expectations change because starts can now be late. Use Sol's sequence as a test.
3. **P2-C, silent-mode boundary.** No write is applied at or after `silent_from`, including through `settle`/`drain` during a Stop at the boundary. Application eligibility respects the last produced sample. Test: a write exactly at `silent_from`.
4. **Note.** Extend the late-command wiring test end to end: the `REnd` send is executed, the brain handles `LateCommand`, and a restart or end-now follows. Keep the focused check as well.
5. **Swarm-style walk scheduler.** Sol reviewed it and found it sound under constraints. Each seed draws three positive weights, one each for Tick, environment delivery and actor work (brain, S, R, clock, guard). Its walk then picks among the **currently enabled** events with those weights. It must not change windows, enabling conditions, transitions or oracles, and mandatory-input blocking stays.
   - Record the weights with the seed in every failure and witness report, and make replay reproduce them.
   - Missing declared witnesses, incomplete runs and exhausted walk budgets stay failures.
   - Then run every case's sweep at 2,000 seeds, and the seeded configuration at horizon 96 at 2,000 seeds. Report the witnesses still unreached, with the seed counts.
   - If some remain unreached, run 10,000 seeds for those cases only, and report them.
6. Rows: one per behaviour change from E1-167 on. Retarget or remove the rows that enforced the rejected behaviour, with reasons. Append to `mutations.json` without re-serializing it.

## After
- Run the default brain suite in debug, with its wall time, and clippy `--all-targets -- -D warnings`.
- In release, run each micro `_full` variant once (2 × 10^6 states, 180 s): verdict, states, missing witnesses.
- Shrink and classify any new failure: (a) world or harness, (b) core, (c) plan gap. Fix (a) with a test and a row. For (b)/(c), add an ignored counterexample test and report it. Change no core decision beyond these points.

## Build and test (owner: never put build or test load on this PC)
- Every cargo command runs through `python3 plan/maintenance/tools/remote/rdev.py -- …` on sim02, one at a time, in the foreground, each under 10 minutes. Never run cargo build, test or clippy locally. The only local cargo command is `cargo fmt --all`.
- Then run `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `python3 plan/maintenance/tools/rules.py --base HEAD`. Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout.

## Report (at most 30 lines)
- Each point: its test, the fix `path:line` and its row(s).
- The cases whose behaviour changed because starts can now be late.
- The micro `_full` table.
- The sweep results: witnesses unreached at 2,000 seeds, and at 10,000 where you ran it.
- New counterexamples with their classification.
- The default suite's count and wall time, and the clippy result.
