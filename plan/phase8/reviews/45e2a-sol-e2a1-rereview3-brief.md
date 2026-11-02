# Brief for Sol: third re-review of E2a-1 (the guard redesign), read-only

You are the independent reviewer of the repository at ~/local_work/Ez-SDRv4. You are read-only: edit nothing, format nothing, run no cargo command that writes into the tree, and change no git state. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.**

## Inputs
- Your second re-review: `~/.cache/ezsdr-fixes/2026-11-01-n45/sol-e2a1-rereview2.md`.
- The orchestrator's design decision after three majors on the guard's recovery: `dispositions-sol-e2a1-code.md`, section "Re-review 2".

The implementer reports these changes:
- **One guard.** `guard_after_panic`, `guard_minimal` and `release_brain_gate` are deleted. One straight-line `guard_routine` in `brain/threads.rs` runs from the unwind handler, never through `run_ops`. It:
  1. acquires `G` unless it is held;
  2. sets `F` unless it is set;
  3. releases `G`;
  4. calls `mark_lost` unless the device is already marked;
  5. ends the brain and routes the terminations.

  It passes the DST guard's `BOp` points in the same order.
- `publish_step`'s dedupe in `shared.rs` is reverted.
- **Tests:**
  - `s5c_a_panic_at_each_brain_operation_still_runs_the_guard`;
  - `s5d_injected_panics_cannot_reach_the_guard`;
  - `s5e_a_lifecycle_call_waiting_for_the_gate_sees_the_failure`, forced;
  - `s5f_free_run_no_close_after_the_failure`;
  - S10b, now a literal event list.
- **Rows:** E2-10 re-anchored; E2-11, 39, 42 and 43 retargeted; E2-47 and E2-48 new.

The code is uncommitted (`git diff HEAD` plus the untracked files in `crates/ezsdr-radio-uhd/src/brain/`).

## Check
1. Are the P1 (I14 after the minimal recovery) and the P2 (E2-44 reaches the pre-ack assertion) closed? Give the evidence at `path:line`.
2. **The new routine.**
   - Is its order equal to the DST guard's order in `world.rs`, and to note 45 §4.5 procedure 4: `F` before `mark_lost`, and `F` published under `G`?
   - Is it safe when the panic happened while the brain held a cell (scratch-swap restoration) or `G`?
   - Can it block forever: `G` held by an R that waits for `L`, while the routine waits for `G`? If it can, is that the bounded-shutdown qualification of note 45 §4.7, or a new hang?
   - Is any leftover of the old recovery still there?
3. Do the new tests observe what they claim? Do the retargeted and new rows mutate what their names say, with deterministic killers?
4. Overall: is E2a-1 now ready to close, after the orchestrator's remote row run? Name anything that blocks it.

## Output
- Verdict: PASS or REVISE.
- A table with one row per item.
- New findings P1/P2/P3. A P1 comes with a minimal sequence and its observable wrong outcome.
- At most 50 lines.
- End with "Not checked:".
