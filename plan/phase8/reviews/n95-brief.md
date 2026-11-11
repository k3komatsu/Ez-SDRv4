# Brief — #95 and #94: a transmit time that does not fit the clock is refused at receipt, on both Providers

Work tree: **/home/komatsu/local_work/Ez-SDRv4-n95** (git worktree, branch `work/n95` from `main` `aa478bc`). Work only there; other trees belong to other tasks. Read first: AGENTS.md (§3, §7, §9 "Maintainability", mutation and build rules), `gh issue view 95`, `gh issue view 94`, `gh issue view 92` (context: a panic in uhd-control is not handled — do not fix #92 here), and the rules involved (search `design/` for MR-16, UR-21, RM-15, RM-37 and the switch-receipt rules; `crates/ezsdr-radio/src/staging.rs` `repeat_boundary`; `crates/ezsdr-mock-radio/src/lib.rs` around the lines #95 names; `crates/ezsdr-radio-uhd/src/provider/control.rs` around the lines #95 names).

Risk: not risky (no queue, lifecycle or ownership change; the UHD Module already refuses at receipt, the Mock is brought to the same answer). No test plan, no self-review; the orchestrator reviews, then full mutation, then Sol's code review.

## Goal

1. **#95**: a `TxBurst` (and any other transmit Action the same code path handles) whose time does not fit the transmit SampleClock is refused at receipt on the MockRadio with its id, the way the UHD Module does (`COMMAND_REJECTED`, the Action's id, a reason naming the rule), and the Run goes on — instead of `ModuleError::rejected(…)?` ending the Run. Cover every `?` site #95 lists and any other overflow `?` on the same receipt paths (search). Keep each reason text consistent with the UHD's where the rule is the same.
2. **#94**: `repeat_boundary` uses checked arithmetic and returns `Option`; `None` becomes a receipt refusal of the switch on both Providers ("the time does not fit the clock", naming the rule), never a panic or a wrapped value. Check every caller of `repeat_boundary` and any sibling helper in `staging.rs` with the same `+ period - 1`/multiply shape (list them; fix those reachable from a user's tick).
3. **Parity**: the two Providers give the same answer for the same Action — add paired tests (the existing paired parity rigs in `crates/ezsdr-acceptance/tests/parity.rs` or the Mock/UHD test files, whichever already compares receipt refusals) for: a burst at `ticks = 2**62`; a switch at `ticks = i64::MAX`; and one ordinary booking right after each, which must still be served (the Run goes on). Unit tests for `repeat_boundary` at the edges (None at overflow; unchanged results elsewhere).
4. **Text**: correct any `design/` rule (MR-16 and others) that says an overflowing time fails the Run on the Mock; the refusal is the documented answer. Keep the rule format of each spec (AGENTS §2).
5. **Mutation rows** for the new refusals and the checked arithmetic (Rust token anchors, unique `old`, compiling `new`, killer named and checked by applying the mutant in a copy outside the repository).

## Rules (binding)

- Builds and tests only through `python3 plan/maintenance/tools/remote/rdev.py --host sim02 -- <cmd>` run from /home/komatsu/local_work/Ez-SDRv4-n95 (sim03 is running another task's gate). The shell tool stops a command at 120 s unless you pass a longer timeout: give rdev runs an explicit long timeout (e.g. 3600000 ms) and wait in the foreground; never leave a command running in the background when you report.
- Never run mutate.py or remote_mutate.py; never apply a mutant inside the tree. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base aa478bc` must report 0 problems.
- No test may depend on when a thread runs (plan/phase8/37-test-thread-timing.md); prefer stepped/frozen rigs and observed records.
- `cargo fmt --all`, then `--check`. `rdev --host sim02 -- cargo test --workspace` once at the end; the Python suite only if Python is affected.
- Test temp files on /tmp; new files mode 644; Python with PYTHONDONTWRITEBYTECODE=1.
- Do not edit handoff.md, AGENTS.md, KNOWN_LIMITATIONS.md (unless an entry describes this behaviour — then report it, do not edit), plan/*/00-overview.md, governance.rs, bench files.
- Do not commit, push or `git add`.

## Report

/home/komatsu/.cache/ezsdr-fixes/2026-11-12-n95/report.md; final message: its summary. Include: every changed site (file:line) with before/after behaviour, the sibling helpers checked, the tests added, the rows and their checked killers, design text changed, every behaviour change, and anything in this brief that proved wrong.
