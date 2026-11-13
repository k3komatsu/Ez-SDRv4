# Note 51 stage 2c — briefs (2026-11-13)

## brief-plan

# Brief — note 51 stage 2c: test plan (read-only)

Read-only run. Trees: the main tree /home/komatsu/local_work/Ez-SDRv4 holds branch `work/n51-2b` (stage 2b, under its gate, landing next on top of main `3f1f87f`; `git -C /home/komatsu/local_work/Ez-SDRv4 diff 3f1f87f..work/n51-2b`); the worktree /home/komatsu/local_work/Ez-SDRv4-e3 is at main. **Do not edit, format or build in either tree**; probes only in a copy outside them (`rsync -a --no-times --exclude .cargo --exclude target /home/komatsu/local_work/Ez-SDRv4/ /tmp/n51-2c-probe/` — the main tree has 2b's committed state on its branch; `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; delete the copy at the end). You are the stage 2c implementer (OpenCode Sonnet 5.5 #high). Write the plan to /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2c/test-plan.md.

Stage 2c (plan/phase8/51-test-timing.md §7.2): "`receipts`（Q10），`receipt_class`，`late_arrival_verdict` の作り直し（今の fault の guard を残す。閾値は試験側の literal），fake の予行の assert，（Q15，Q16，Q17）" — **risky** (a production record, hardware tests' expectations): an audit table, a fidelity review, the next bench. Read note 51 (§5.2–5.3, §6, §7.2, §10–§11 Q10, Q15–Q17 and the decisions after them, the "段 2a の後" and "段 2b の後" paragraphs), plan/phase8/51-tables.md (§3 R table incl. the R2 window end `J(e)`, §5 timing-INCONCLUSIVE list, §6 rows), the 2b records (plan/phase8/reviews/n51-2b-* in the main tree's branch) for the 13 deferred tests, and the code: crates/ezsdr-radio-uhd/src/provider/control.rs (the receipt read, ~:491), tests/common/{iii,n47s5}.rs (`receipt_class`, `late_arrival_verdict`, `stop_booking_verdict`), and the hardware tests that share those trials.

Plan, as the earlier stages' plans: what `receipts` records (where, in the Manifest or a Provider section, its schema impact — user-visible? say so: the owner decides what the Manifest records), the verdict rules rebuilt on it, each of the 13 deferred tests with its new expectation and what it proves, the hardware tests whose expectations change (AGENTS.md §9: the audit table test / expectation / still right / fixed / cannot tell), the fault guards kept, the thresholds as test-side literals, Q15–Q17 as decided, the failing-test-first set (the 2a sweep's injections that fail each today), mutation rows (Rust token anchors), sub-steps, and the owner decisions this stage needs (list them, with a recommendation each).

Report: the path of test-plan.md, a short summary, and the owner decisions.

## brief-plan2

# Brief — note 51 stage 2c: amend the test plan after Sol's review (read-only)

Read-only run, as before (no edits/format/build in the trees; probes only in a copy outside them, deleted at the end). Main tree /home/komatsu/local_work/Ez-SDRv4 is at main `89fd3f7` (2b landed); it is idle except for a watchdog. You are the stage 2c implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2c/: your test-plan.md, sol-plan.md (Sol: FAIL, five majors, four minors) and plan-decisions.md (the orchestrator's decisions: all adopted, M1 as a 3 ms post-receipt window `DEVICE_LEAD + POLL`, M2 keeps the Kernel-log witness).

Amend test-plan.md: a "Changes after Sol's review" section at the top, and the body updated to be the plan to implement: M1's window with the UR-14/UR-21 sentence you propose (quote it), the stepped-rig boundary test and Sol's 3–5 ms test, F1/F2 re-measured with 3 ms in a copy; M2 reverted to the log witness; M3's fault order with the two counterexample tests; M4's verdict change with the hardware tests it changes in the audit table; M5 derived or deferred with the reason; the four minors. Keep the owner decision on the `receipts` row shape as an open item (the orchestrator recommends one row per Action × stream, kinds tx_burst / update_parameter / stop / start_rx, none for refusals before the read).

Report: the path, a summary of what changed, and anything INFERRED.

## brief-plan3

# Brief — note 51 stage 2c, test plan v3 after the owner's §5.3b decision (plan only, no builds)

Main tree /home/komatsu/local_work/Ez-SDRv4 at 4327a66 plus one uncommitted edit (plan/phase8/51-test-timing.md §5.3b). Read-only for the tree: do not edit any file in the tree, and **no cargo builds or tests at all** (a timing-sensitive hardware bench is running on this machine). You are the implementer (OpenCode Sonnet 5.5 #high). Write only in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2c/.

Read: plan/phase8/51-test-timing.md (whole note; §5.3b is the new governing rule and replaces §5.3 priorities 3–6 and §5.3a's permission table — §5.3a's N2–N5 survive), and in this directory: test-plan.md (v2, written under the withdrawn P7 rule), plan-decisions.md (all sections; the "Withdrawn" section and the two owner decisions at the end bind), sol-plan.md, fable-5-3b.md.

Task: rewrite the stage 2c test plan as test-plan-v3.md (keep test-plan.md as is). It must:
1. Implement §5.3b exactly: one Fake-side helper that reads the three lateness evidences (TIME_ERROR cause late via `iii::time_error_causes`; `LogEntry.time` past the test's literal window; a test-side read before submit past the window, late side only), called before the shared verdict and before Fake-only asserts; on evidence, write the timing marker and skip that trial; multi-trial tests skip the marked trial, run the rest, fail at the end. N7 stays judged by the unchanged `late_arrival_verdict` and never reaches the predicate.
2. Drop everything built on P7 / receipt classes / the permission table / the `receipts` Manifest row (no Manifest change in 2c).
3. Carry from v2 what still holds (Sol's M3 fault-priority counterexamples as pure tests; M4 long-close FAIL in the shared verdict; M5 rx-restart derivation or deferral to stage 3; the minors adopted in plan-decisions.md) and say for each v2 item: kept / changed / dropped, with the reason.
4. Include Fable's M2 (the mutation baseline goes through `judge`; list the rows whose filter names a rehearsal, with evidence), M3 (F2: C = 1 000 in the shared trial with an audit of the hardware tests that share it; F3: `stop_next_trial`; the rule "20 serial runs with 0 markers before a rehearsal's assert is switched" as a step in the plan, run at implementation time, not now), M4.
5. List the tests the predicate covers (path:line), and for each sequence: the test, what it asserts, how it observes it, and the wrong behaviour that fails it. Mutation rows (Rust token anchors) for the predicate, the marker path, the baseline judge, and a direct-rig filter for every row now killed through the hardware guard (`iii.rs` guard) that would become ERROR — list those rows with evidence.
6. List the other note-51 text §5.3b's last paragraph names, with the replacement wording, for the implementation step.
7. Mark hardware/timing claims VERIFIED (path:line or measured log) or INFERRED.

Report: the path and a short summary (what changed from v2, open questions for the orchestrator). Do not commit; do not start reviewers.

## brief-plan4

# Brief — note 51 stage 2c, test plan v4 after Sol's review (plan only, no builds)

Same rules as brief-plan3.md (read-only tree, NO cargo builds/tests/probes: a hardware bench is running; write only in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2c/). You are the implementer (OpenCode Sonnet 5.5 #high).

Read sol-plan3.md (Sol: FAIL, 2 majors, 4 minors) and plan-decisions.md (its last two sections bind: the orchestrator's decisions on your open questions, and on Sol's findings — all adopted). Copy test-plan-v3.md to test-plan-v4.md and amend v4: a "Changes after Sol's review of v3" section at the top, the body updated (M1 guard per site + finish() with negative fixtures; M2 admission check inside the Rehearsal path with the integrated negative case; clause 3 removed everywhere; the late_at_device test; N2's own verdict line; the 20-run check's counting), rows and the note-51 text list updated. Also apply decisions 3–7 of the open-questions section.

Report: the path and a short summary.

## brief-plan5

# Brief — note 51 stage 2c, test plan v5 (plan only, no builds)

Same rules as brief-plan3.md (read-only tree, NO cargo builds/tests/probes: a hardware bench is running; write only in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2c/). You are the implementer (OpenCode Sonnet 5.5 #high).

Read the last section of plan-decisions.md (the Opus re-check of v4: two new majors R1, R2 and a minor, all adopted). Copy test-plan-v4.md to test-plan-v5.md and amend v5: a short "Changes after the Opus re-check of v4" section at the top, the body, tests, negative fixtures and rows updated for R1, R2 and the minor. Report the path and a short summary.

## brief-impl

# Brief — note 51 stage 2c implementation (sub-steps per test-plan-v5.md)

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-2c` at 7a2199f (main 4327a66 + the §5.3b text commit). You are the implementer (OpenCode Sonnet 5.5 #high). Self-review is waived (Sonnet-implemented step); Sol reviews the code at the end.

**A timing-sensitive hardware bench is running on this machine until about 13:30–14:00.** Until the orchestrator tells you otherwise: NO local cargo builds, tests or clippy. Build and test only through `python3 plan/maintenance/tools/remote/rdev.py -- <command>` (sim02/sim03), and mutation rows only through `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <list> sim02:8,sim03:8 <ids...>`, **in the foreground** (never wait for a notification). Local `cargo fmt --all` is fine. The "20 serial runs with 0 markers" checks run through rdev too.

Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2c/: test-plan-v5.md (the plan to implement), plan-decisions.md (every section binds; the last one adds the re-check residuals: count `_verdict(`/`_outcome(` outside spans, `if let [a, b]` for p70, identifier-boundary macro names, N2's late drops through `late_arrival_verdict`), sol-plan3.md. First fold the last section of plan-decisions.md into test-plan-v5.md (short section at the top), then implement in the plan's sub-steps.

Rules:
- Per sub-step: tests green for the crates you touched (via rdev), the sub-step's mutation rows killed (remote_mutate, foreground), `cargo fmt --all`, `python3 plan/maintenance/tools/rules.py --base 4327a66` 0 problems. Then write `sub-step-<n>-report.md` in the directory above and **exit** (the orchestrator commits each sub-step on `work/n51-2c` and relaunches you). Do not commit.
- Every planned sequence gets its test, every test its killing row; the hardware audit table for every hardware test whose expectation changes (AGENTS §9).
- The note-51 text changes the plan lists (§5.3b's last paragraph) go in the sub-step the plan assigns. Edit only what the plan names; never edit /home/komatsu/local_work/Ez-SDRv4-e3 (another implementer works there).
- Test temp files on /tmp. Kill processes only by PID.

Report per sub-step: files changed, tests run (command + result), rows run (ids + outcome), plan deviations with reasons, anything INFERRED.

## brief-impl2

# Brief — note 51 stage 2c implementation, next sub-step

Continue from brief-impl.md (same rules: no local cargo builds/tests until the orchestrator says the bench is over — rdev.py and remote_mutate.py in the foreground only; do not commit; never edit /home/komatsu/local_work/Ez-SDRv4-e3). Sub-step A is committed on `work/n51-2c` (your report: sub-step-A-report.md; accepted as is, including the BASELINE FAILED re-run count). Implement the plan's next sub-step (B), write `sub-step-B-report.md` and exit.

## brief-impl3

# Brief — note 51 stage 2c implementation, next sub-step

Continue from brief-impl.md, with one change: the bench is over, so run cargo tests and clippy **locally** now (not rdev); mutation rows still only through remote_mutate.py in the foreground; the "20 serial runs" checks also run locally. Do not commit; never edit /home/komatsu/local_work/Ez-SDRv4-e3. Sub-steps A and B are committed on `work/n51-2c` (your reports sub-step-A/B-report.md; B accepted as is, including rows 46–48 and the guard's `calls` column).

Implement the plan's next sub-step (C1, including what B's report leaves to it: the literal uses in n47s5.rs, wrapping the nine tests, PairTrial, the Stop trial's n1_stamp, the hardware.rs record line, the real-tree wiring test and its table, p70's recount). Write `sub-step-C1-report.md` and exit. If C1 is small and the plan's next sub-step fits in the same run, you may do it too, with its own report section.

## brief-impl4

# Brief — note 51 stage 2c implementation, next sub-step (C2 and what follows)

Continue from brief-impl3.md (local cargo tests; rows via remote_mutate.py foreground; do not commit; never edit Ez-SDRv4-e3 or Ez-SDRv4-a1). Sub-steps A, B, C1 are committed on `work/n51-2c`; C1 accepted (see the last line of plan-decisions.md: the 4/24 parallel front markers are acceptable — serial runs were clean; keep rows N51-2C-50..53). Implement C2 (switch the rehearsals from report-only to withholding, the 2a sweep) and, if it fits, the plan's remaining sub-steps (the note-51 text changes, KL/handoff as the plan says), then run the full mutation lists once through the sim queue, foreground: `~/.cache/ezsdr-fixes/tools/simq env REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8`, and settle every failing row. Write `sub-step-C2-report.md` (and one section per further sub-step) and exit.

## brief-impl5

# Brief — note 51 stage 2c, sub-step C3 (N7 under injected delay)

Continue from brief-impl4.md (local cargo tests; rows via remote_mutate.py foreground; do not commit; never edit Ez-SDRv4-e3 or Ez-SDRv4-a1). C2 is committed on `work/n51-2c`. Read the last section of plan-decisions.md: wrap both held-tail N7 cases in `Rehearsal::trial` with a `Lateness` that leaves out only C's own expected late drop; update the guard table (`direct`/`outside` counts), its fixtures, the note-51 text that says N7 never reaches the predicate (§2.4 of the plan, §5.3b's N7 sentence), and rows N47S5-192..194 if their killers move; add a test that a clean N7 run (only C late) is judged, not skipped (a row that makes the exclusion too wide, e.g. leaving out every drop, must die). Re-run the sweep points now-60, now-200, submit-60, submit-200 for the held-tail test, 20 serial runs of it, the touched rows, and `cargo test -p ezsdr-acceptance --test governance`.

You may offload mechanical work to Go DeepSeek yourself (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`), at your judgment. DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries): read its diff and run the tests before you accept it. Run it in the foreground, one at a time, and do not touch the tree meanwhile; list what you offloaded in the report.

Write `sub-step-C3-report.md` and exit.

## brief-fix1

# Brief — note 51 stage 2c, fix round 1 (Sol code review)

Continue from brief-impl5.md's rules (main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-2c`; local cargo tests; rows via simq + remote_mutate.py in the foreground; do not commit; never edit Ez-SDRv4-e3 or Ez-SDRv4-a1). Read /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2c/sol-code.md and the last section of plan-decisions.md there: fix M1, M2 and the minor as written there. Each fix gets its test first (red before the fix where practical) and its killing row (new ids N51-2C-58…). Then: the four sweep points for the M1 test and the held-tail test, 0 plain failures; 20 serial runs of the touched rehearsals; `cargo test -p ezsdr-radio-uhd --test fake`, `-p ezsdr-acceptance --test governance`, the lib tests of ezsdr-radio-uhd; the touched and new rows; `cargo fmt --all`; `rules.py --base 4327a66`; `cargo clippy -p ezsdr-radio-uhd -p ezsdr-acceptance --all-targets -- -D warnings`. Update note 51's text if a sentence becomes false.

You may offload mechanical work to Go DeepSeek yourself (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`), at your judgment. DeepSeek makes mistakes: read its diff and run the tests before you accept it. Foreground, one at a time, not touching the tree meanwhile; list what you offloaded.

Another lane (#67, worktree Ez-SDRv4-e3) builds and tests on this host at the same time: if the 20 serial runs show markers, check `uptime` and rerun them once the load drops; report both.

Write `fix1-report.md` (files, tests and results, rows and outcomes, deviations, INFERRED) and exit.

## brief-fix2

# Brief — note 51 stage 2c, fix round 2 (minor-only)

Same rules as brief-fix1.md (main tree, branch `work/n51-2c`, fix 1 committed as 21b7683; local cargo; rows via simq + remote_mutate.py in the foreground; do not commit; never edit Ez-SDRv4-e3 or Ez-SDRv4-a1; build with no CARGO_TARGET_DIR override — the tree's .cargo/config.toml decides, and touch your changed files before building if a filter matches 0 tests). Fix m1–m4 and the incomplete lines exactly as the last section of /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-2c/plan-decisions.md says (the re-check's details are quoted there). Run: `cargo test -p ezsdr-radio-uhd --test fake`, `-p ezsdr-acceptance --test governance`, row N51-2C-67 and any row whose anchor you touched, `cargo fmt --all`, `rules.py --base 4327a66`, clippy for the two crates.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits across files, renames, new tests that follow an existing pattern, mutation rows, text updates — and keep your own effort for design, review and the hard parts. What to hand over is your call. DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries): read its diff and run the tests before you accept it. Run it in the foreground, one at a time, and do not touch the tree meanwhile; list what you offloaded in your report.

Write fix2-report.md and exit.
