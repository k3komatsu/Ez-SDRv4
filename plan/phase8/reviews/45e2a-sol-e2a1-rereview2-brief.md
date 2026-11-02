# Brief for Sol: second re-review of the E2a-1 fix round, read-only

You are the independent reviewer of the repository at ~/local_work/Ez-SDRv4. You are read-only: edit nothing, format nothing, run no cargo command that writes into the tree, and change no git state. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.**

## Inputs
- Your re-review `~/.cache/ezsdr-fixes/2026-11-01-n45/sol-e2a1-rereview.md`: REVISE, with P1-A, P2-B and P2-C.
- The dispositions: `dispositions-sol-e2a1-code.md`, its section "Re-review", items A–D. D is the row E2-39, which survived the remote run.

The implementer reports these changes:
- **A:** `guard_after_panic` and `guard_minimal` at `brain/threads.rs:363-420`, plus `release_brain_gate`. `shared.rs` records `F` and `mark` once each. The tests are `s5c_a_panic_at_each_guard_operation_still_runs_the_guard` and `s5d_a_panic_during_the_recovery_does_not_escape`.
- **B:** the composite row E2-44 in `core.rs`, killed by S10b.
- **C:** `publish_recomputes_when_acceptance_lands_between_compute_and_write`, with a `before_write` hook. E2-37 is retargeted to the `threads.rs` call site. E2-45 and E2-46 are new.
- **D:** E2-39 now points at the forced `s5c`, because the free-run killer was timing-dependent.
- **E2-10** is re-anchored.

The code is uncommitted (`git diff HEAD` plus the untracked files in `crates/ezsdr-radio-uhd/src/brain/`).

## Check
1. Are P1-A, P2-B and P2-C closed? Give the evidence at `path:line`.
2. The sequences next to each fix:
   - **The guard's minimal path:** is `F` before `mark_lost` kept? Does every executor still terminate? Can releasing the brain's gate let an executor's lifecycle call slip in after `F` (I14)?
   - **The once-only recording of `F` and `mark`:** can it hide a second, real failure report?
   - **The `before_write` hook:** test-only? Is it free of effects in free runs?
3. Is each new row's anchor unique, and does each row's mutated program match its name? Would the named test kill it deterministically?

## Output
- Verdict: PASS or REVISE.
- A table with one row per item.
- New findings P1/P2/P3. A P1 comes with a minimal sequence and its observable wrong outcome.
- At most 50 lines.
- End with "Not checked:".
