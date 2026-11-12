# Note 51 stage 1d — briefs (2026-11-12)

## brief-plan

### Brief — note 51 stage 1d, phase 1: the test plan (no code)

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1d` from `main` `bfd59d3`. Read only; write only the plan file below. You are the implementer (OpenCode Sonnet 5.5 #high); stage 1d is **risky (small)**: the test plan comes first and Sol reviews it.

Read first: AGENTS.md §3, §7, §9; plan/phase8/51-test-timing.md (§3 contracts, §4.1, §4.4 the `drop(frozen)` table — types N, G, W1, W2, H —, §7.2 rows 1a–1d and 4, the paragraphs "段 1b の後" and "段 1c の後", §8, §11, the appendix incl. Q6); plan/phase8/51-tables.md (the rows for `src/provider/{tx,rx}.rs` and `tx/tests/{e1,o1}.rs`); the 1a, 1b and 1c records plan/phase8/reviews/n51-1a-*, n51-1b-*, n51-1c-* (the `HostClock::drive()` / `Driver` API, the one wait rule R1/R2, the drop-order tests, the infra marker); the #76 shape the table names for G (search the reviews and issues for #76: `gh issue view 76`); crates/ezsdr-radio-uhd/src/host.rs, src/provider/test_support.rs, src/provider/tx.rs, src/provider/tx/tests/e1.rs.

## Stage 1d (note 51 §7.2: "G の 3 か所を guard，W2 は試して決める")

The three G sites (§4.4: after `drop(frozen)` the test thread runs the owner's code itself) get the guard (#76's shape) so their verdict does not depend on when the test thread runs. The two W2 sites (`tx.rs` around the old line 6676 and `tx/tests/e1.rs:88`: wake a participant sleeping 150 ms, `shutdown`, `join`) are tried: a rewrite that removes the sleeping helper and the join so the guard fits, kept only if it works, otherwise left as they are with the reason. The N, W1 and H sites stay (W1 with a guard would hang). The line numbers in §4.4 are from before E2b–E3b and 1c: find the sites again by content and give the search.

## The plan (write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1d/test-plan.md)

1. The 15 `drop(frozen)` sites today, each with path:line, its type (N/G/W1/W2/H) re-checked against the code, and what 1d does with it; any site or `Frozen` use the table missed that has the G shape (search and give the search).
2. For each G and W2 site: whether its verdict depends today on when the test thread runs (a diagnostic delay injection in an out-of-tree copy, note 51 §5.4: diagnostic only), the failing-first evidence, the change and the test that pins it.
3. W2: the rewrite tried (in the copy), whether it works, the decision with evidence.
4. Mutation rows (Rust token anchors), each naming a killer that fails on an assertion, not on a bound or a race; existing rows whose filter or killer changes.
5. Text 1d makes false (note 51 §4.4 and §7.2, the tables, the tests README).
6. Sub-steps, if any, each leaving the tree green.

Rules: no code edits in the tree; probes only in a copy outside the repository built **locally** (`rsync -a --no-times --exclude .cargo` to /tmp/opencode/<name>, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`), rebuilt from source every time; not on sim02/sim03; no commit. Final message: a summary, and anything in note 51, the tables or the 1a–1c records the code contradicts.

## brief-impl

### Brief — note 51 stage 1d, implementation (sub-steps 1d-A, 1d-B, 1d-C in one run)

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1d` at `bfd59d3`. You are the stage 1d implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1d/: brief-plan.md, your test-plan.md, sol-plan.md (Sol: PASS WITH CHANGES) and plan-decisions.md (all three minors adopted, 1d-C′ deferred to stage 4). First add a "Changes after Sol's review" section at the top of test-plan.md; then implement 1d-A, 1d-B and 1d-C as the amended plan states (the helper `handed_over`, its unit tests, the 4 G sites, the `ur_28_…` slow step with the new clock-equality assertion, the 3 W2 rewrites, rows N51D-01…03, the re-runs), and the text of §5 (note 51 §4.4/§7.2/§11, the tables, the tests README as the plan lists) with the counts corrected. Not 1d-C′.

Rules: tests must not depend on when a thread runs (note 51 §3); existing assertions unedited unless the plan names the change. **Build and test locally**, not on sim02/sim03; probe copies rebuilt from source. Timing-dependent tests 20 runs (the G and W2 tests and the helper tests). Run rows N51D-01…03 and every existing row whose code or killing test you touched (the plan's re-run list without C′'s four) through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim03:4 <ids>` and **wait for it to finish before reporting**; report each row's failing assertion (test and message). Never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base bfd59d3` = 0. Add a "Changes in 1d" section to test-plan.md. No commit.

Report: `git diff --stat`; what you did per sub-step; rows and outcomes; deviations with reasons.

## Orchestrator decisions on the plan

### Stage 1d — orchestrator decisions on the plan (2026-11-12, decided by the orchestrator)

Sol's plan review (sol-plan.md): PASS WITH CHANGES, no major, three minors — all adopted.
- m1: the helper unit test's 10 s `recv_timeout` setup bound writes the infra marker (`test_support::infra`) on `Timeout` only (not `Disconnected`); N51D-02's kill stays the clock-equality assertion; no `"kill": "bound"`.
- m2: `ur_28_…`: add an exact clock-equality assertion right after the deliberate slow step and before `tx.pass()`, so the guard pin observes that the clock did not move during the stall; describe N51D-03's actual failing assertion accurately; keep the failing-first evidence with the original re-freeze in the plan.
- m3: negative diagnostic claims read "no dependence observed at these injection points/durations".
- 1d-C′ is **deferred to stage 4** (no `Frozen` hand-over there; not a complete timing closure). The four rows that apply only with C′ (P1298, P1302, P1306, P1312) are not touched.
- One implementation run and one local commit for A+B+C (about 90 lines); the per-sub-step review is waived for this small step and the self-review is waived (Sonnet implementer, owner 2026-11-03); Sol's code review remains (AGENTS §9 step 7). Recorded as a deviation in the dispositions.
- Note 51's counts are corrected in the text (§4.4: 17 sites, 4 N / 4 G / 5 W1 / 3 W2 / 1 H; the `rig._frozen` spelling; §11 W2 VERIFIED; "guard (#76 の形)" → the `drive()` hand-over helper).
