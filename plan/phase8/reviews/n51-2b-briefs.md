# Note 51 stage 2b — briefs (2026-11-13)

## brief-plan


Main tree /home/komatsu/local_work/Ez-SDRv4 at `main` (`829b7b0`, stage 2a landed). You are the stage 2b implementer (OpenCode Sonnet 5.5 #high). This run writes **a plan only**, to /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2b/test-plan.md; no edits in the tree (probes in a copy outside it: `rsync -a --no-times --exclude .cargo --exclude target <tree>/ /tmp/n51-2b-probe/`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; delete it at the end). Another lane works in the worktree ~/local_work/Ez-SDRv4-e3 (#99: crates/ezsdr-radio/src/staging.rs, crates/ezsdr-radio-uhd/src/provider/tx.rs and its tests): stay out of those files.

Stage 2b (plan/phase8/51-test-timing.md §7.2): "R 表のとおり R1・R2 を直結へ移す／assert を外す。rows の filter を付け替え，同じ mutant が死ぬことを確かめる" — risk "場面ごと". Read note 51 (§3, §5.2–5.3, §7.2, the "段 2a の後" paragraph), plan/phase8/51-tables.md §3 (the R table, the "Planned move" column) and §6 (rows whose filter is a Run-based rehearsal or a real-clock test), plan/phase8/reviews/n51-2a-sweep.md (the 15 tests whose verdict changes under injected delays: 14 `fake`, 1 `uhd` `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end`), plan/phase8/reviews/n51-2a-dispositions.md (incl. the three allow-listed time-reading tests that drive a Run, and the `support/mod.rs` `session_run_with` note), and crates/ezsdr-radio-uhd/tests/README.md.

Plan:
1. The set: every test whose planned move is 2b's (a move to the direct rig, or an assert removed/relaxed that does not need stage 2c's `receipts`), each with its line now, its class, why its verdict depends on when the test thread acts (the sweep's evidence), the planned move, and what the moved test still proves (the behaviour, the assertion, the wrong behaviour that fails it). Tests whose move needs `receipts` go to 2c: list them with the reason. `rx_start_lead_group_runs_on_the_fake` was left "unclassified until 2b's trace": classify it.
2. The three allow-listed time-reading tests that drive a Run and the `support/mod.rs` `session_run_with` path: 2b or 2c, with the reason.
3. For each move, the mutation rows whose filter selects the test (tables §6): the new filter, and how you will show the same mutant still dies on an assertion (run through remote_mutate.py later; here only the list).
4. Risk per scene (AGENTS.md §9 step 1): which moves change what a test proves (risky) and which are mechanical; sub-steps.
5. A failing-test-first check where it applies: the sweep's injection that fails the test today and passes after the move.

Report: the path of test-plan.md and a short summary (counts per scene, the 2c list, risk).

## plan-decisions


- Scope as planned: 5 moves in 2b (`ur_21_a_refused_configure…` split into a Run half and a new `Direct` window test; `ur_21_s18`, `ur_23_a_burst_after_a_stop…`, `ur_29_a_loss_found…` one booking each untimed / `send_asap`; `ea_16_untimed_repeat_after_stop…` reads the Stop's own log stamp); `ur_29_a_silent_stream` and `ur_33_a_shutdown…` lose their time reads and their allow-list rows; `ur_21_a_descheduled…` stays (stage 3); the `session_run_with` gap closed by a scan rule (`assemble_on` outside `uhd.rs`); 13 tests to 2c (need `receipts`), incl. `rx_start_lead_group_runs_on_the_fake` (R2).
- Included (the implementer's question): `bounded()` and the `entered.recv_timeout` handshake write the infra marker on their timeout (tables §4.3), so a hang there is a bound, not a kill. Rows whose only kill was that hang then need `"kill": "bound"` or a retarget — list them.
- Risk: not risky (test code only; no production change). No Sol plan review; the audit table for the split and `ea_16` stays in the plan; Sol's code review at the end. One implementation run, one local commit (small step), sub-steps A–D reported separately; self-review waived (Sonnet).
- Record corrections: the 2a sweep's changed set is 14 tests (13 `fake` + 1 `uhd`): fix the text in note 51's "段 2a の後" paragraph and plan/phase8/reviews/n51-2a-dispositions.md ("14 `fake`" → "13 `fake`", "15" → "14").

## brief-impl


Main tree /home/komatsu/local_work/Ez-SDRv4 at `main` (`829b7b0`); create nothing on git (no branch switch, no commit: the orchestrator commits). You are the stage 2b implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2b/: brief-plan.md, your test-plan.md, plan-decisions.md. Another lane works in the worktree ~/local_work/Ez-SDRv4-e3 (#99): do not touch crates/ezsdr-radio/src/staging.rs, crates/ezsdr-radio-uhd/src/provider/** or design/.

Implement the plan's sub-steps A–D as decided (incl. the `bounded()` / `entered.recv_timeout` infra marker in A, and the record corrections in D: note 51's "段 2a の後" paragraph and plan/phase8/reviews/n51-2a-dispositions.md say 14 tests, 13 `fake` + 1 `uhd`). The audit table for the split and `ea_16` goes into test-plan.md ("Changes in implementation"). Rows: retarget the filters the plan lists; every row whose only kill becomes a bound gets `"kill": "bound"` or a retarget, listed.

Rules: tests must not depend on when a thread runs (note 51 §3); moved or new timing-dependent tests 20 runs; rerun the 2a sweep's injection (plan/phase8/reviews/n51-2a-sweep.md: the configs that failed each moved test) and show each moved test now passes under it. **Build and test locally.** Never a mutant in the tree (rows only through remote_mutate.py); kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; clippy `-D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 829b7b0` = 0. At the end run the full lists: `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8 > /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2b/impl-mut.log 2>&1` (the other lane may hold the sims; simq waits): **do not end your run until it has finished and you have read its result**; settle every row that is not `killed` / `killed (bound)` / the known `phase7/G09`, re-run those to impl-mut2.log.

Report: `git diff --stat`; per sub-step what changed; each moved test, what it proved before and proves now, and its result under the injection; rows retargeted or marked bound; the full-list result.

## brief-finish


The previous implementer run (brief-impl.md in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2b/) stopped while waiting for a mutation-run notification that never came. Its changes are uncommitted in the main tree /home/komatsu/local_work/Ez-SDRv4 (branch `work/n51-2b`, base `829b7b0`): `git diff --stat`. Its full mutation run finished: impl-mut.log ends `2382/2383 passed; not passed or missing: ['phase7/G09']` (G09 is the known survivor), so no row needs settling. You are the stage 2b implementer (OpenCode Sonnet 5.5 #high).

Do: read brief-impl.md, plan-decisions.md, test-plan.md (its "Changes in implementation" and audit table) and the diff; check every item of brief-impl.md is done (sub-steps A–D incl. the `bounded()` / `entered.recv_timeout` infra marker, the record corrections to 14 = 13 `fake` + 1 `uhd` in note 51 and n51-2a-dispositions.md, the injection re-run for each moved test, the 20 runs); finish anything missing; then `cargo fmt --all --check`, `cargo test --workspace`, clippy `-D warnings`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 829b7b0` = 0. Do not start another full mutation run; if you change code a row selects, run only those rows (`REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <id> ...` and wait for it in the foreground). No commit.

Report: `git diff --stat`; per sub-step what changed; each moved test, what it proved before and proves now, and its result under the injection; rows retargeted or marked bound; the full-list result.
