# Brief: note-45 experiment E2b, fix round 2 — measure the design, not the forcing harness

You are the implementer (AGENTS.md §9). Repository `~/local_work/Ez-SDRv4`, branch `main`. Edit in place: you are the only editor of the tree. Read AGENTS.md first. Do not cd into `v3/`. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`** and do not search for it.

## Where things are
- Briefs, in order. The definitions and verdict of the first brief stay binding unless a later brief changes them.
  - `brief-e2b.md`, including its Amendments section;
  - `brief-e2b-fix.md`, parts A and B: done, uncommitted in the tree.
- Sol's reviews:
  - `sol-e2b-code.md`;
  - `sol-e2bA-code.md`;
  - **`sol-e2bB-code.md`**: the `# REVISE — Part B` section at the end, with 2 × P1, 3 × P2 and 1 × P3, and a ranked diagnosis.
- Dispositions: `dispositions-sol-e2b-code.md`.
- Results: `e2b-results/` (round 1) and `e2b-results-2/` (round 2).
- All the paths above are in `~/.cache/ezsdr-fixes/2026-11-01-n45/`.

## Why this round is different (orchestrator's decision)
Three review rounds have now found the same root cause: the runtime being measured is the trace-forcing test runtime of E2a. Each round has found more cost that note 45 does not ask for:
- per-sample reconstruction and cloned history (round 1);
- a broadcast on one shared scheduler condvar at every acquisition and publication, which wakes the brain, S, the clock and the timer about 200,000 times a second (round 2);
- an unbounded per-call log in `CheckingDevice`;
- extra cell acquisitions per block.

The speed verdict must measure what a production runtime of this design would do. So, in **measurement runs only**:
1. **Targeted wake-ups.** Outside forcing, a thread is woken only when an input it waits on changes:
   - the brain on its inbox;
   - S on its FIFO;
   - `R_k` on its FIFO or its stop flag (a receiving R does not wait);
   - the timer on a new or earlier deadline;
   - the clock reader on its own schedule.

   Give each actor its own wait (a condvar per actor, or per queue), notified by the push that concerns it. The forcing path (`force.rs`, trace replay) must stay exactly as it is: the S1 replays, S2–S10 and every E2-* row keep passing. If the cleanest way is for `Sched` to choose per-actor notification when no controller is installed, do that. Do not write a second copy of the executor loops (AGENTS.md §9 Maintainability 1): the loops stay one code path, and only the wait and wake primitive differs.
2. **No observation history on the hot path.** `CheckingDevice` keeps its no-cell-held assertion (I10) but records no per-call log in measurement runs (`threads.rs:~125-126`, `model_device.rs:~336-348,417-418`). No other per-block vector or record may grow either. Search for them.
3. **The receive loop's own cost.**
   - Evaluate `loop_next` once per iteration.
   - Use the design's two cell sections per block: the view, then the recheck and write.
   - Make no allocation in `consume_samples` when nothing is pending (`rules.rs:~383`).

   Equivalence is checked as in fix round 1, part A2:
   - the brain suite;
   - S1;
   - the 2,000-seed walks (W7b needs 10,000 seeds);
   - `s1b_full`;
   - conformance.
4. **Known costs left alone, to be listed in the report:**
   - FakeDevice's DSP history scan, `device.rs:~548-553`, O(H) per block. It is common to both owners, and FakeDevice is out of scope.
   - The brain's linear result and disposition searches, `core.rs:~380-393,602-605`. They cost per update, not per block.

   Say whether either one limits a condition.

## Sol's Part B findings, all adopted
- **P1 B3:** restore the timestamp-gap count (`Reader::missing`) as the missing-samples loss, including leading and trailing gaps without counting a gap twice. Reconcile publication loss separately. Today's derived discard needs a justified boundary; otherwise report it as derived and not compare on it.
- **P1 per-block tracing:** item 2 above.
- **P2 B1:** check each recorded `apply` return against its own `e + 10 ms` deadline, revisiting the final outstanding entries too. Persist the booking identity and effective instant actually observed, with the requested and ingress timestamps. Do not pair a request with a stray apply.
- **P2 B5:** record the actual rejection reason (bound, loss, cut, and so on) as bounded per-update aggregates, not one record per attempt. A loss rejection is not a bound hit.
- **P2 B2:**
  - take the apply and DSP snapshots after a successful stop and drain;
  - failed joins, panics or an exit that is not orderly make a run unclean;
  - each repetition's `pass` includes cleanliness.
- **P3 B6:** remove the frequency fallback from the DSP labels.

## Diagnostics first, then the verdict runs
1. **Throughput.**
   - Run the 50 s no-update trial at 50 MS/s × 500, new runtime, once before item 1 and once after, on sim03 through rdev.
   - Record per-second blocks, plus the voluntary and involuntary context switches of each thread from `/proc/<pid>/task/*/status`, sampled each second.
   - If `perf` works in the container, add a sampled CPU profile. Say whether it works.
2. **Latency.** One bursty 1 MS/s diagnostic run, after item 1, with bounded per-update stamps at:
   - the release sent by the timer;
   - the brain's dequeue;
   - the command pushed to S;
   - S's dequeue;
   - the `apply` entry and return.

   Report each stage's p50 and p99.
3. **Then the verdict runs, as before:**
   - the rate trial for both owners;
   - the four conditions with ABBA and BAAB;
   - the diagnostics;
   - the fault runs.

   Write them to `e2b-results-3/`, run them on sim03 one at a time, and record `uptime`.

## Rules
- Run `cargo fmt --all` and `cargo clippy -p ezsdr-radio-uhd --all-targets -- -D warnings`, both clean.
- Run the brain suite. Run each timing-dependent test you add or change 20 times.
- Add rows for:
  - the targeted wake (a mutant that wakes no one, or the wrong actor, must hang or fail a test, with a bounded timeout);
  - the B1 expiry check;
  - the B5 reason;
  - B2 cleanliness.

  Append them as text and never re-serialize `mutations.json`. Then run `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `python3 plan/maintenance/tools/rules.py --base HEAD` (0 problems).
- Never run mutate.py or remote_mutate.py. Never git commit, push, stash, reset or checkout. Kill processes by PID only.
- Out of scope: production code, FakeDevice behaviour, `design/`, the Kernel, schemas and Python. The definitions and the verdict thresholds stay as they are.
- Write one line to `~/.cache/ezsdr-fixes/2026-11-01-n45/e2b-fix-progress.txt` as each item is done.

## Report (at most 30 lines)
- What changed for items 1–3, and the equivalence results.
- The two throughput diagnostics side by side.
- The latency stage table.
- Per-condition verdict tables: owner, p50, p99, p99.9, max, count, the three loss counts, failures and the verdict.
- Bound hits with their reasons.
- The fault runs.
- The known costs from item 4 and whether they limit.
- Rows added.
- Lines changed.
- Anything you think is wrong in this brief.
