# Brief for Sol: second narrow check of N1 (E2a-2), read-only

You are the independent reviewer of the repository at ~/local_work/Ez-SDRv4. You are read-only: edit nothing, build nothing, and change no git state. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.**

Your narrow check `sol-e2a2-check.md` found two P2s. The fixes reported, recorded in plan §17.4:

**N1**
- `R_k` are now participants of the fake's manual clock. They are spawned through `HostClock::spawn` via `force::spawn_on` in `brain/force.rs`, as production's threads are.
- The test advances only by its own `host().sleep` of one tick. `parked()` and `settle()` are deleted.
- **A hang was found and fixed along the way.** A participant idle on something other than the clock froze the clock, so participant idle waits now go through `Sched::idle`, which waits on the fake's clock and is woken by `Sched::notify`.
- **A 20 s watchdog** in `run_fake` ends the process if participants wait on each other.
- **New test:** `s6c_time_cannot_pass_a_runnable_executor`, with row E2-60.

**E2-55** now uses `.front()`.

## Check
1. Is N1 closed?
   - Can wall time still change the simulated history? Look at how `Sched::idle` waits and wakes: can a notify be lost?
   - Is the clock frozen whenever any `R_k` (and S, the brain, K, if they touch the fake) is runnable?
   - Do the brain, S and K need to be participants as well? Production spawns uhd-control, uhd-rx and uhd-clock through `host.spawn`.
2. **The watchdog.** Is ending the whole test process acceptable, or should it fail only that test? Can it fire under heavy load on correct code?
3. Does `s6c` observe what it claims, and does E2-60 break it? Does E2-55 now compile and fail `s9i`?
4. Readiness for the full gate.

## Output
- Verdict: PASS or REVISE.
- Findings P1/P2/P3 with `path:line`.
- At most 40 lines.
- End with "Not checked:".
