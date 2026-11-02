# Brief: note-45 experiment E2b, fix round 3 — close the open measurement findings and run one discriminating diagnostic

You are the implementer (AGENTS.md §9). Repository `~/local_work/Ez-SDRv4`, branch `main`. Edit in place: you are the only editor of the tree. Read AGENTS.md first. Do not cd into `v3/`. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`** and do not search for it.

## Where things are (all in `~/.cache/ezsdr-fixes/2026-11-01-n45/`)
- Briefs: `brief-e2b.md` (with its Amendments), `brief-e2b-fix.md`, `brief-e2b-fix2.md`. Everything they asked for is done and uncommitted in the tree, except what this brief lists.
- **Sol's latest review: `sol-e2b-fix2.md`**, the `# REVISE` section at the end. The earlier reviews and `dispositions-sol-e2b-code.md` give the history.
- The latest results are in `e2b-results-3/`.

## The result as it stands (orchestrator, from Sol's review)
- 1 MS/s, spaced and bursty: **PASS**, clean (new p99 about 80 µs, today's about 1,050 µs).
- 50 MS/s × 500: **supported**. Both owners pass trial candidate 2 at 99,999.98 blocks/s with zero overflows and a backlog of about 0.05 ms. The 62.6k and 75.3k figures belong to candidate 0, 200 MS/s × 2,000. The previous report's "unsupported" was a misreading; correct it in the README of `e2b-results-3/`. With timed updates the new runtime **FAILs** this condition.
- The cause is VERIFIED in the code (Sol). A successful call keeps its in-flight slot until R observes the consumption in samples (`core.rs:~626-671,910-917`), whereas today's command slot expires at device time (`provider/control.rs:~659-666,688-689`). A host receive backlog therefore turns into settings lateness at the bound of 4. That the backlog in the verdict runs triggered it is INFERRED: the verdict files carry no synchronized backlog samples.

## Do
1. **The discriminating diagnostic.** Run one paired high-rate diagnostic, spaced and then bursty, at 50 MS/s × 500 with 10,000 updates, new runtime only, on sim03 through rdev:
   - **bound 4**, as designed;
   - **bound = the device's real capacity**. Take the DSP FIFO depth that FakeDevice enforces for a timed DDC value (32, per `e2b-speed-facts.md`) or the command-queue depth (16), whichever binds first for these commands. Find it in `device.rs` and cite it.

   Make the bound a parameter of the measurement runtime only, with the default unchanged. Record synchronized per-second samples of:
   - the fake backlog;
   - in-flight occupancy per chain;
   - retirements per second;
   - for each update: apply entry and return, the application tick, and the retirement time.

   This run is **diagnostic**. It does not replace the bound-4 verdict, and it is not evidence for the original design. State plainly whether the backlog excursions coincide with the bound hits.
2. **FakeDevice scan cost (isolated diagnostic).** Measure how much of R's per-block time the fake's DSP-history scan (`device.rs:~548-553`) takes as H grows. Compare a 50 s no-update trial with a trial after H timed values have been applied up front (for example 10,000 far-past entries), and give blocks/s and R's CPU. Do not change FakeDevice. If it cannot be done without changing it, say so.
3. **Sol's open P2 findings:**
   - **B3:** the verdict's missing-samples count comes from the reader: leading and in-between gaps plus the trailing gap, counted once each. It is not `tap_stats.missing` (`speed/runs.rs:~403-407,522-537`). Publication drops stay a separate number. Rerun only the verdict comparisons that this changes. These are the four conditions; the high condition FAILs on failures regardless, so state whether any loss comparison flips.
   - **B1 booking identity:** persist the identity of each request's booking and its effective instant from the owner itself, not by pairing instants (`speed/mod.rs:~626-656`, `speed/runs.rs:~385-388,468-489`). If today's owner exposes no booking identity without a production change, say so, and label the instant pairing as inferred in the output.
   - **Wake tests:** in `s15a` and its siblings, prove that each actor has reached its wait before the push, using the `Sched` waiting probe or an equivalent, so that a wrong-wake mutant cannot pass by reading a queue already filled. Add a test of an earlier booking while the timer already sleeps on a later deadline.
   - **Claims without a row:** add tests for the snapshot-after-stop order (a delayed shutdown), the `unlogged` wiring in `start_measured` (an integration assertion that no per-call log grows in a measured run), and the first-view reuse (a non-forcing test with a stale view, to show the recheck catches it). Single evaluation is accepted by inspection.
4. **Rows:** add a row for each new test where a mutant is meaningful, and re-check that E2-84 to E2-89 now kill through the tightened tests. Append them as text and never re-serialize the file.

## Rules
- Run `cargo fmt --all` and `cargo clippy -p ezsdr-radio-uhd --all-targets -- -D warnings`, both clean.
- Run the brain suite. Run each new timing-dependent test 20 times.
- Run `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `python3 plan/maintenance/tools/rules.py --base HEAD` (0 problems).
- Never run mutate.py or remote_mutate.py. Never git commit, push, stash, reset or checkout. Kill processes by PID or container ID only. Run filters precisely: do not launch every ignored test.
- Out of scope:
  - production code;
  - FakeDevice behaviour;
  - `design/`;
  - the verdict thresholds and definitions;
  - the core's retirement rule, which is the design question for the orchestrator and the owner. Do not change it.
- Write one line to `e2b-fix-progress.txt` as each item is done. Write the results to `e2b-results-4/`.

## Report (at most 25 lines)
- The paired diagnostic: per-second backlog against bound hits, failures and p99 at bound 4 and at the device's capacity.
- The scan-cost diagnostic.
- The corrected loss numbers and whether any comparison flipped.
- B1 identity.
- The new tests and rows.
- The suite results.
- Lines changed.
