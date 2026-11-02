# Brief for Sol: fourth narrow check of the E2-60/E2-61 killers, read-only

You are the independent reviewer of the repository at ~/local_work/Ez-SDRv4. You are read-only: edit nothing, build nothing, and change no git state. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.**

Your third narrow check `sol-e2a2-check3.md` found one P2. The fix, recorded in plan §17.6:
- **`HostClock::participants()`**, `#[cfg(test)]` in `src/host.rs`, takes one snapshot under the clock's lock: the clock reading, and each registered participant by name with its busy state and manual deadline.
- **`s6c` and `s6d`** use `held_alone`. It asserts that the held actor is registered, then waits for a snapshot in which that actor is busy and every other participant (except the test body and the sleeper) is idle or on a deadline not yet due, and requires the clock reading to be unchanged. There is no wall-time inference.
- **E2-60 and E2-61** fail immediately, because the actor is not registered.
- **`every_runtime_thread_of_a_fake_run_is_a_participant`** is new.

## Check
1. Is the P2 closed? Are the killers deterministic under any load?
2. Does `held_alone` have a wait that could hang or pass vacuously?
3. Does `participants()` change any production path? Is it `cfg(test)`-only?
4. Is E2a ready for the full gate?

## Output
- Verdict: PASS or REVISE.
- Findings P1/P2/P3 with `path:line`.
- At most 25 lines.
- End with "Not checked:".
