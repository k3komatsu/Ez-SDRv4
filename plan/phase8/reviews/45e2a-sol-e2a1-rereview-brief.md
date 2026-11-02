# Brief for Sol: re-review of the E2a-1 fix round, read-only

You are the independent reviewer of the repository at ~/local_work/Ez-SDRv4. You are read-only: edit nothing, format nothing, run no cargo command that writes into the tree, and change no git state. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.**

## Inputs
- Your code review: `~/.cache/ezsdr-fixes/2026-11-01-n45/sol-e2a1-code.md`, verdict REVISE with 1 P1, 4 P2 and 1 P3.
- The orchestrator adopted all six, with decisions, in `~/.cache/ezsdr-fixes/2026-11-01-n45/dispositions-sol-e2a1-code.md`. Its Q1 section records the acceptance-instant finding for E3, with no change to production.
- The implementer's report of the fix round:
  1. Atomic batch removal (`threads.rs:421-436`) and `s3c_command_routed_during_failure_cleanup_is_abandoned_not_lost`.
  2. The guard covers the step and every op (`threads.rs:341-380`), with `s5b_panic_inside_a_brain_operation_runs_the_guard_and_restores_the_cells`.
  3. S5 waits for real thread completion (`Sched::is_exited`), and the discarder ends on `Shared.brain_gone`.
  4. `s10b_no_new_epoch_command_is_routed_before_the_quiesce_ack` reads `Sink.routed`. The implementer did not add a single-guard row and claims that `retiring` and `busy` are redundant guards here. Check that claim.
  5. `declared` is the union over seeds, with `s1b_fails_when_a_declared_witness_is_missing`.
  6. The publish is split into `rules::publish_compute` (outside the cell) and `rules::publish_write` (inside, after `Cell::unchanged_since`), with a retry.
- Rows: E1-13 retargeted, E2-04 and E2-14 changed, E2-36…E2-41 new.
- The Q1 test labels became "intervention A/B".
- The code is uncommitted: `git diff HEAD` plus the untracked files in `crates/ezsdr-radio-uhd/src/brain/`.

## Check
1. Is each finding closed? Give the evidence at `path:line`.
2. The sequences next to each fix:
   - **For 1:** any other place where a FIFO is snapshotted and later cleared, in S, R or the discarder. Can a command still be lost or abandoned twice?
   - **For 2:** a panic in the guard path itself, and lock poisoning of the cell mutexes after an unwind. Do the executors still see coherent cells?
   - **For 3:** can an executor still hang when the brain dies while it waits for `L` or `G`?
   - **For 6:** can the retry loop livelock, for instance against a Stop booking that changes the revision on every round? Does `unchanged_since` compare every field the compute read? Does the forced replay still pin the snapshot and write points?
3. Do the new tests observe what they claim, and do the new and changed rows break them?

## Output
- Verdict: PASS or REVISE.
- A table with one row per earlier finding.
- New findings P1/P2/P3. A P1 comes with a minimal sequence and its observable wrong outcome.
- At most 70 lines.
- End with "Not checked:".
