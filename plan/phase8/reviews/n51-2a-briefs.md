# Note 51 stage 2a — briefs (2026-11-12/13)

## brief-plan


Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-2a` from `main` `42f5031`. Read only; write only the plan file below. You are the implementer (OpenCode Sonnet 5.5 #high); stage 2a is **risky** (the boundary between the fake and the hardware it stands for: shared trials and `hardware.rs`): the test plan comes first and Sol reviews it.

Read first: AGENTS.md §3, §7, §9 (incl. "Hardware tests and bench values": the audit table); plan/phase8/51-test-timing.md — §3 (contracts), §5 (Run rigs: §5.1–§5.4, especially §5.4 the wrapper, the 12 creation sites, the scan, delay injection as diagnostic), §6 (§6.2 classify bounds before returning control, the two declarations, the RS-8a sequence, unwind rule; §6.3 marker; §6.4 the e2e fixtures — 2a adds fixtures 4, 4b, 6, 7), §7.2 row 2a, §8, §11, the appendix decisions (Q3, Q10, Q11, Q15–Q17, (i)) and the paragraphs after stage 0, 1b, 1c, 1d; plan/phase8/51-tables.md (R table, the bound list, §5.4's lists); stage M and 1a–1d records plan/phase8/reviews/n51m-*, n51-1a-* … n51-1d-*; crates/ezsdr-radio-uhd/tests/common/mod.rs (`session_with` ~494, `spec_run` ~460), tests/common/{iii,n47s5,measurement,…}.rs (the trials shared with hardware), crates/ezsdr-radio-uhd/tests/hardware.rs, crates/ezsdr-acceptance/tests/uhd.rs, crates/ezsdr-server/tests/protocol.rs (`server.handle(`), plan/maintenance/tools/gate.py and mutate.py (the e2e stage and marker reading), the Kernel's `RunHandle` (crates/ezsdr-kernel/src/coordinator/run.rs, ending.rs, manifest.rs).

## Stage 2a (note 51 §7.2)

"Run の rig の包み（共有 trial と `hardware.rs` を含む。`session_with` 以外の生成 12 か所も包みの生成に替える），全 method の検査（§6.2：上限の分類は常に，宣言は 2 種類）と fixture 4・4b・6・7，診断の注入点，`protocol.rs` の `handle` 1 か所，走査（`connect(`／`start_spec_run(`／`RunHandle` を含む）". Compile of `hardware.rs` is checked at the gate (`uhd` feature build); its behaviour at the next bench. The Kernel gets nothing (§6.2).

## The plan (write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/test-plan.md)

1. The wrapper: its type(s) and where they live (and how the acceptance crate gets the same wrapper — §5.4 leaves that to this brief: recommend one, with the alternative rejected), its methods (only those used today, with the search that shows it), what each method checks on return (§6.2 steps 1–3), the two declarations' API, `finish()` returning the Manifest, `Drop` and unwind, how a test that forgets `finish()` is caught (§6.2 "残るもの": scan or `#[must_use]` — decide).
2. Every Run creation site today (the 12 of §5.4 plus `session_with`, re-found by search on the current tree, the search given), and each test that expects a non-deadline end (the 21 + 4 + 3 of §6.2, re-found): its declaration.
3. The shared trials and `hardware.rs`: what changes in type only; the AGENTS §9 audit table (hardware test, expectation, still right / fixed / cannot tell); the default-suite rehearsals that pin the same boundaries.
4. `protocol.rs`: the one `handle` function and its injection point.
5. The scan: what it forbids, where, the per-fn and per-file allow lists (§5.4), where it runs (a test? rules.py?), its self-test.
6. e2e fixtures 4, 4b, 6, 7 (§6.4): each fixture's sequence and expected verdict through `gate.py`/`mutate.py`.
7. For each check, the test and the wrong behaviour that fails it; failing-first evidence where a sequence exists today (§6.2's KC-21a and RS-8a sequences: show today's false `killed` in an out-of-tree copy if practical).
8. Mutation rows (Rust token anchors; exact substrings for Python), each with a killer that fails on an assertion; existing rows whose filter or killer changes (2a changes many tests' plumbing: list the rows whose filter names a test you change).
9. Diagnostic delay injection at the end (§5.4: injection points × {5, 20, 60, 200 ms}, report only).
10. Text 2a makes false (note 51, the tables, the tests README, AGENTS/handoff wording if any).
11. Sub-steps, each leaving the tree green (this is large: propose an order, e.g. wrapper + checks on `session_with` first; then the 12 creation sites; then shared trials and hardware.rs; then protocol.rs; then the scan; then fixtures).

Rules: no code edits in the tree; probes only in a copy outside the repository built **locally** (`rsync -a --no-times --exclude .cargo` to /tmp/opencode/<name>, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`), rebuilt from source; not on sim02/sim03; no `mutate.py`/`remote_mutate.py`; no commit. Final message: a summary, the sub-step list with sizes, and anything in note 51, the tables or the records the code contradicts.

## plan-decisions


Sol's plan review (sol-plan.md): FAIL, three majors, three minors — all adopted.
- **M1, join before T0 is not exempt.** The wrapper classifies a Provider join (`did not join within`) as a bound unconditionally; the `now >= t0` exemption is dropped. The healthy tests that finish before T0 with an RX segment begun ahead of its origin hit the open product bug #67 (uhd-rx waits for a distant origin at `Provider::stop`; streamers leak; the fix waits on #56's bench). For each: advance past T0 before finishing when that keeps the test's subject; otherwise a narrow, named declaration (e.g. `Expect::KnownIssue67` / `.known_join_issue(67)`) that exempts the join of `uhd-rx-*` only and says so in a one-line comment citing #67; it is removed when #67 is fixed. List those tests in the plan.
- **M2, method-return tests must not be satisfied by `Drop`.** Each check test catches the panic around the one method call (the `Run` owned outside the closure, e.g. `catch_unwind(AssertUnwindSafe(|| run.advance_to(..)))`) and asserts that call panicked (and the marker presence/absence); a later `Drop` must not satisfy it. Rows 06–09 must die on those assertions.
- **M3, a bound cut in the background.** Add the scenario (a Policy end whose `stop_early` on the data thread hits a join timeout, `paced.rs:133-148`, while the test calls only `events()`/`state()`). Resolve the observation boundary: if a read-only method can see `CleanedUp`, it checks too (cheap: `state()` is one lock); if the Manifest cannot be obtained without settling, narrow the guarantee explicitly in note 51 §6.2 and the README ("every settling call and `finish`; a read-only return checks the state only"). No Kernel change.
- Minors: the scan forbids raw `coordinator::connect`/`start_spec_run` (incl. a renamed import) outside `run.rs` and allows `Run::connect(`; self-tests for the allowed form, a renamed import, whitespace and multi-line forms. Fixture setup waits on an observed-entry handshake with an infra-marked setup bound, not 700 ms; fixture 8 (join at `finish`) is kept. `protocol.rs` injection after `server.handle` returns, consistently, with a test.
- `Expect::Any` only on individually named observation-only bench loops (`hw_perf_rx_rates`, the B8 leads); it still rejects every bound and reports the termination; not through shared helpers; FakeDevice checks that a non-deadline end is reported and the loop continues while a bound aborts; B9 keeps its named DEVICE_LOST expectation.
- Sub-steps as the plan's §11 (8), each a separate run and local commit; sub-step 2 is atomic. Self-review waived (Sonnet implementer, owner 2026-11-03); Sol's code review at the end.

## Sub-step 1 → 2 (orchestrator, 2026-11-12 ~21:50)
- Sub-step 1 committed (7d233eb → rebased 11211ea onto e449397, C2). Rows N51-2A-01…07 killed. Full lists not run at sub-step 1 (dead code + 7 new rows): they run at sub-step 2's end, which switches every caller.
- Sub-step 1's M3 choice (Drop finishes a live Run even while unwinding) accepted subject to sub-step 2 showing finish() cannot panic during unwind (double panic = abort), else guarded.

## Sub-step 2 (orchestrator, 2026-11-12 ~23:10)
- `run_stage` (measurement/multichannel.rs) declares `Expect::Any`, not `Refused`: it is an observation-only stage that records a Policy end (`run_ended_early`); with `Refused` a device lost mid-stage on the bench would panic instead of being recorded. Bounds still fail. Sub-step 6's scan allows `Any` in `run_stage` besides `hw_perf_rx_rates`/`hw_b8_leads` (now `perf_rx_rate_trial`/`b8_lead_trial` in common/mod.rs).
- P880: `"kill": "bound"` accepted (its only kill is the teardown meeting #67; goes away with #67) rather than three more `rx_join_issue_67()` declarations.
- Accepted: `capture_markers`, `effects`, the `catch_unwind` guard in `Drop` (finish() panics only on a broken Kernel invariant), `rx_join_issue_67()` on `spec_run_ended`'s failure path, N92-1's narrowed filter with N51-2A-21 (bound) keeping the submit-waits coverage.
- The `hardware` binary is checked only by the gate's hw-build.

## Sub-steps 6–7 (orchestrator, 2026-11-13 ~00:50)
- The three allow-listed time-reading tests that drive a Run (`ur_29_a_silent_stream_is_a_lost_device`, `ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one`, `ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx`) stay allow-listed with their reasons in 2a; moving them to a rig is stage 2b/2c's work (they read the real clock on purpose or need a real-clock rehearsal).
- The sweep's 15 tests whose verdict changes under injected delays (14 `fake`, 1 `uhd`) are the input of stages 2b/2c (note 51 §5.2), not 2a findings.
- `rig_delay` in `common/infra.rs` (one copy, protocol.rs includes it) accepted; the scan's size (410 lines, a lexer) accepted.

## brief-impl1


Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-2a` at `42f5031`. You are the stage 2a implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/: brief-plan.md, your test-plan.md, sol-plan.md (Sol: FAIL, three majors, three minors) and plan-decisions.md (all adopted, with the orchestrator's resolutions). First add a "Changes after Sol's review" section at the top of test-plan.md resolving each point (M1's list of tests that hit #67 and how each is handled; M2's call-level catch; M3's scenario and the observation boundary you choose, with evidence; the minors; `Expect::Any`), and update §11's sub-step contents if they move. Then implement **only sub-step 1** as amended: `crates/ezsdr-radio-uhd/tests/common/run.rs` and its unit tests, nothing that switches callers yet; its rows if the plan gives sub-step 1 rows.

Rules: tests must not depend on when a thread runs (note 51 §3); pure tests once, timing-dependent 20 runs. **Build and test locally**, not on sim02/sim03; probe copies rebuilt from source. Run the rows you add through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids> > /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/impl1-mut.log 2>&1` — the sims may be busy with another lane's gate (simq waits, possibly 30+ minutes): **do not end your run until that command has finished and you have read its result**; poll its PID if your tool call times out. Never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 42f5031` = 0. Add a "Changes in sub-step 1" section to test-plan.md. No commit.

Report: `git diff --stat`; the amendments in brief; what sub-step 1 did; rows and each failing assertion; deviations with reasons.

## brief-impl2


Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-2a`. Sub-step 1 is committed (`11211ea`, `run.rs` and rows N51-2A-01…07), rebased onto `e449397` (main now holds note 49 C2, which changed `crates/ezsdr-radio-uhd/tests/common/iii.rs` and other tests: recount the `RunHandle` sites; §2's numbers are from `42f5031`). You are the stage 2a implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/: test-plan.md (with its "Changes after Sol's review" and "Changes in sub-step 1"), plan-decisions.md, sol-plan.md, and your sub-step 1 report at the end of impl1.log.

Implement **only sub-step 2** of test-plan.md §11: the switch in `ezsdr-radio-uhd` — `session_on`, `start_spec` and its wrappers through `Run`; every `RunHandle` in `tests/common/**`, `tests/fake.rs`, `tests/hardware.rs` → `Run`; the creation sites of §2 that are in this crate; the Policy, refusal, subject, #67 (`rx_join_issue_67`) and bench-loop declarations; J317/J318 `"kill": "bound"`; the method / Ended-mode / `Drop` / unwind / `Any` rehearsal tests and their rows. Not `ezsdr-acceptance`, `protocol.rs`, the fixtures, the scan or the injection (sub-steps 3–7).

Points to settle in this sub-step, with evidence in your report:
1. **`Drop` while unwinding** now calls `finish()` (changes note 51 §6.2). Show whether `RunHandle::finish()` (and what it calls) can panic; a panic in `Drop` during unwinding aborts the whole test binary. If it can, guard it (no `finish()` while panicking, or `catch_unwind`) and keep the marker. A test: a check test that panics inside a live `Run` ends with that test's own failure, not an abort.
2. **#67 declarations**: your first run prints the joined thread names for the five tests; confirm each is a `uhd-rx-*` thread. A test whose join is another thread is not covered by `rx_join_issue_67`: stop and report it.
3. No test's expectation changes except by a declaration the plan names. A test that newly fails on a bound you did not plan is a finding to report, not to declare away.

Rules: tests must not depend on when a thread runs (note 51 §3); pure tests once; timing-dependent new tests 20 runs. **Build and test locally**, not on sim02/sim03; `cargo test -p ezsdr-radio-uhd --test fake` and the `hardware` binary type-check as the plan says (no libuhd here; the gate's `hw-build` builds it). Never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base e449397` = 0. Add a "Changes in sub-step 2" section to test-plan.md. No commit.

Mutation: at the end run the **full lists** (this sub-step changes the type every UHD integration test uses): `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8 > /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/impl2-mut.log 2>&1` (about 15 min plus queue; the other lane may hold the sims). **Do not end your run until that command has finished and you have read its result**; poll its PID if your tool call times out. Settle every row that is not `killed` / `killed (bound)` / the known `phase7/G09`: retarget, or report with the reason.

Report: `git diff --stat`; what changed; the three points above with evidence; new rows and each failing assertion; the full-list result; deviations with reasons.

## brief-impl34


Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-2a` at `de0a5fc` (sub-steps 1 and 2 committed; base `e449397`). You are the stage 2a implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/: test-plan.md (with the "Changes after Sol's review", "Changes in sub-step 1" and "Changes in sub-step 2" sections), plan-decisions.md (including "Sub-step 2": `run_stage` declares `Expect::Any`), and your sub-step 2 report at the end of impl2.log.

Implement **sub-steps 3 and 4** of test-plan.md §11, in that order: 3 — `crates/ezsdr-acceptance/tests/uhd.rs` through `Run` (`#[path]` include, sites 9–11, the `Declared` parameter on `session_with_layout`, `effective`/`state` as methods); 4 — `protocol.rs` (the `handle` fn, its call sites, the injection point after `handle` returns with its own small `rig_delay` reader as the plan says). Not the fixtures, the scan or the delay injection in `run.rs` (sub-steps 5–7). Keep each sub-step's changes separable in your report (the files of each).

Rules: as in brief-impl2.md (tests must not depend on when a thread runs; pure tests once, timing-dependent new tests 20 runs; build and test locally; never a mutant in the tree; kill by PID; `cargo fmt --all`; `cargo test --workspace` green; clippy `-D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base e449397` = 0; no commit). No test's expectation changes except by a declaration the plan names; a test that newly fails on a bound you did not plan is a finding to report.

Mutation: at the end run the full lists (the switch reaches the acceptance and protocol tests): `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8 > /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/impl34-mut.log 2>&1` (the other lane may hold the sims; simq waits). **Do not end your run until it has finished and you have read its result**; settle every row that is not `killed` / `killed (bound)` / the known `phase7/G09`, then re-run those rows (ids as separate arguments) to `impl34-mut2.log`.

Add "Changes in sub-steps 3 and 4" to test-plan.md. Report: `git diff --stat`; what changed per sub-step; new rows and each failing assertion; the full-list result and how each non-killed row was settled; deviations with reasons; anything needing the orchestrator's decision.

## brief-impl5


Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-2a` at `7fd3415` (sub-steps 1–4 committed; base `e449397`). You are the stage 2a implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/: test-plan.md (with the "Changes after Sol's review", "Changes in sub-step 1" and "Changes in sub-step 2", "Changes in sub-steps 3 and 4" sections), plan-decisions.md (including "Sub-step 2": `run_stage` declares `Expect::Any`), and your sub-steps 3–4 report at the end of impl34.log.

Implement **sub-step 5** of test-plan.md §11: the fixtures 4, 4b, 6, 7 and 8 (kept, plan-decisions), `Stalls`, the `infra_fixtures` tests, `mutate.py`'s `E2E` cases and `lacks`, the self-test cells; fixture handshakes wait on an observed entry with an infra-marked setup bound (Sol's minor). Run the e2e step remotely as the plan says (`remote_mutate.py --e2e`, through `~/.cache/ezsdr-fixes/tools/simq`) and each fixture by hand locally. Not the scan or the delay injection (sub-steps 6–7).

Rules: as in brief-impl2.md (tests must not depend on when a thread runs; pure tests once, timing-dependent new tests 20 runs; build and test locally; never a mutant in the tree; kill by PID; `cargo fmt --all`; `cargo test --workspace` green; clippy `-D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base e449397` = 0; no commit). No test's expectation changes except by a declaration the plan names; a test that newly fails on a bound you did not plan is a finding to report.

Mutation: at the end run the full lists (the fixtures change what the runner reads): `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8 > /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/impl5-mut.log 2>&1` (the other lane may hold the sims; simq waits). **Do not end your run until it has finished and you have read its result**; settle every row that is not `killed` / `killed (bound)` / the known `phase7/G09`, then re-run those rows (ids as separate arguments) to `impl5-mut2.log`.

Add "Changes in sub-step 5" to test-plan.md. Report: `git diff --stat`; what changed; the e2e result; new rows and each failing assertion; the full-list result and how each non-killed row was settled; deviations with reasons; anything needing the orchestrator's decision.

## brief-impl67


Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-2a` at `b2e45b6` (sub-steps 1–5 committed; base `e449397`). You are the stage 2a implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/: test-plan.md (with the "Changes after Sol's review", "Changes in sub-step 1" and "Changes in sub-step 2", "Changes in sub-steps 3 and 4", "Changes in sub-step 5" sections), plan-decisions.md (including "Sub-step 2": `run_stage` declares `Expect::Any`), and your sub-step 5 report at the end of impl5.log.

Implement **sub-steps 6 and 7** of test-plan.md §11, in that order: 6 — the scan in `governance.rs` (`scan`, the allow lists, the self-test, rows 13–18 of the plan as renumbered), with Sol's minor (a renamed or multi-line raw `connect`/`start_spec_run` is caught; `Run::connect(` allowed) and the orchestrator's decision that `Expect::Any` is allowed in `run_stage` (measurement/multichannel.rs) besides `perf_rx_rate_trial` and `b8_lead_trial` and nowhere else; 7 — the delay injection in `run.rs` (`rig_delay`, env parsing, its small pure tests; reuse protocol.rs's reader shape) and the sweep script outside the repository (in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/) with one sweep report there (which tests' verdicts change under injected delays at each point; a verdict that changes is a finding to report, not to fix here). Keep each sub-step's files separable in your report. Not the text (sub-step 8).

Rules: as in brief-impl2.md (tests must not depend on when a thread runs; pure tests once, timing-dependent new tests 20 runs; build and test locally; never a mutant in the tree; kill by PID; `cargo fmt --all`; `cargo test --workspace` green; clippy `-D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base e449397` = 0; no commit). No test's expectation changes except by a declaration the plan names; a test that newly fails on a bound you did not plan is a finding to report.

Mutation: at the end run the full lists (the scan and the injection touch every UHD integration test binary): `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8 > /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/impl67-mut.log 2>&1` (the other lane may hold the sims; simq waits). **Do not end your run until it has finished and you have read its result**; settle every row that is not `killed` / `killed (bound)` / the known `phase7/G09`, then re-run those rows (ids as separate arguments) to `impl67-mut2.log`.

Add "Changes in sub-steps 6 and 7" to test-plan.md. Report: `git diff --stat`; what changed; the sweep report; new rows and each failing assertion; the full-list result and how each non-killed row was settled; deviations with reasons; anything needing the orchestrator's decision.

## brief-impl8


Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-2a` at `ec3bb2b` (sub-steps 1–7 committed; base `e449397`). You are the stage 2a implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/: test-plan.md (§10 and every "Changes in sub-step…" section), plan-decisions.md (all sections), sol-plan.md, and your reports at the ends of impl1.log … impl67.log, plus sweep/report.md.

Implement **sub-step 8**: the text test-plan.md §10 lists, as amended by the later sections and decisions — plan/phase8/51-test-timing.md (Japanese, keep its style: §6.2 the classification rule and the guarantee at method returns, `Drop` finishing a live Run even while unwinding (guarded), the declarations as built (`Expect` Refused/Policy/Any, `join_is_subject`, `rx_join_issue_67` citing #67 — the pre-T0 join of §10 item 1 is #67, already filed: cite it, no new issue), §6.4 the fixtures as built, §5.4 the 82 `handle` call sites, §7.2 row 2a, §8 the two "検出（2a の後）" rows, §10's 2a text, a "段 2a の後" paragraph after stage 1d's, with the sweep's result in one or two sentences pointing to stages 2b/2c), plan/phase8/51-tables.md (§3.1 line numbers, J317/J318 `bound`), crates/ezsdr-radio-uhd/tests/README.md (the wrapper, its checks, `Drop`, `EZSDR_RIG_DELAY_MS`/`EZSDR_RIG_DELAY_AT`, the scan), plan/maintenance/failure-mechanisms.md (`wall-clock-test-oracle` row: one sentence). Do **not** edit handoff.md, plan/phase8/50-remaining-work.md or plan/phase8/reviews/ (the orchestrator writes those). No code changes; if the text shows code that contradicts it, report it instead of changing code.

Rules: no build needed beyond `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base e449397` = 0 (rules.py checks long lines and markers; keep design/ untouched). No commit. Report: `git diff --stat`, what each file now says in one line, anything in the text you could not make true without a code change.

## brief-fix1


Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-2a` at `ae7cf6d` (sub-steps 1–8 committed; base `e449397`). You are the stage 2a implementer (OpenCode Sonnet 5.5 #high). Read /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/sol-code.md (Sol: FAIL, one major, two minors), and as needed test-plan.md, plan-decisions.md and your reports.

Adopted (orchestrator), all three:
1. **Major** — a bound recorded in `termination.also` is invisible: the wrapper judges only `termination.reason` (crates/ezsdr-radio-uhd/tests/common/run.rs ~193-196; the Kernel keeps later failures in `also`, crates/ezsdr-kernel/src/coordinator/ending.rs ~204-225). Judge every termination (primary and `also`) for bound reasons before any declaration is accepted; a declaration never exempts a bound found in `also` (except `join_is_subject` / `rx_join_issue_67` for exactly what they name). Pure test in `judge_end`'s tests: a declared Policy primary with KC-21a in `also` → `Limit("KC-21a")`; and the same for RS-8a / a join. If a real interleaving can be driven deterministically on the manual clock (Sol's sequence: background Policy, then the waiting thread's KC-21a), add it; otherwise say why not. Rows killed by the pure tests.
2. **Minor** — `capture_markers` suppresses unexpected markers: in tests that expect no bound, do not capture; in deliberately bounded tests, capture only the expected bound's marker and let any other marker print (crates/ezsdr-radio-uhd/tests/common/run.rs ~91-105, fake.rs ~8708-8714). A test or row pinning that an unexpected marker still prints.
3. **Minor** — the scan misses a function-value alias (`let create = ezsdr_kernel::coordinator::connect; create(..)`): reject raw constructor references (`connect`, `start_spec_run` as paths, not only calls) outside run.rs; a self-test for the alias; keep `Run::connect(` / `Run::start_spec(` allowed (crates/ezsdr-acceptance/tests/governance.rs ~510-520).

Update note 51 §6.2 / README text if a sentence becomes false (one sentence each).

Rules: as before (tests must not depend on when a thread runs; build and test locally; never a mutant in the tree; kill by PID; `cargo fmt --all`; `cargo test --workspace` green; clippy `-D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base e449397` = 0; no commit). Then run the new rows and every row whose code or killing test you touched: `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <id> <id> ... > /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-2a/fix1-mut.log 2>&1` (ids as separate arguments): **do not end your run until it has finished and you have read its result**.

Report: `git diff --stat`; each fix with path:line; tests and rows with their killing assertion; anything not done and why.
