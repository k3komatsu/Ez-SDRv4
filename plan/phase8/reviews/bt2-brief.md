# Phase 8 lane 1, step bt2 (plan/phase8/50-remaining-work.md §2 row 3): the C7 bench tool, and C2 at 100/200 MS/s — Sonnet drives Go DeepSeek

Main tree `<main tree>`, branch `work/bt2` (base main `715ebd67`). Work only there; never touch `<worktree>` (another lane) or other worktrees. Do not commit, stash, push, reset or switch branch.

**Risk: risky** (AGENTS §9 step 1): it adds a production hook and crosses the boundary between the FakeDevice and the X310 it stands for. So this run has **two phases**:
- **Phase A (this run): design + test plan, no code.** Write `~/.cache/ezsdr-fixes/2026-11-19-bt2/test-plan.md` and stop with a report. The orchestrator has Sol review the plan; you get the adjudicated plan in a fresh run for phase B (code).
- Phase B (a later run): implement against the reviewed plan.

## Goal
1. **C7** (`plan/phase8/bench.md` table row C7 and "### C7"; design `~/.cache/ezsdr-fixes/2026-11-14-mt3/addendum.md` §2.3–2.4 and §3; `decisions.md` lines 23–31 there; #106 changed the cap: a host-fed repeat now resumes after **every** drop, the stutter bound b′ is gone — `plan/phase8/reviews/n106-*`): a Module-level (Session/Run through the real UHD Module) `#[ignore]` hardware test that, on both boards, runs a `start_repeat` (a) forced late at its start and (b) starved mid-play, with the resume margin m swept over 10, 1 and 0.2 ms, ≥ 10 trials per cell, and reports per trial: silence in `[from, R)` heard on the RX loopback, the waveform back at R phase-continuous on the repeat's lattice, the events (`TIME_ERROR { late_at_device }` / `TX_UNDERFLOW { starved }` with `resumed_at`), `stats.tx_resumes` / `tx_gap_samples`, and whether the resumed part was dropped again (R early). It also estimates τ (the host-to-radio delay on the Module's path; addendum §2.3: the drain ends at `D ≤ busy + τ + ε·S + b`) from what the RX capture and the Module's own record show — say exactly which observable gives τ and its resolution. The owner later sets m from C7, so the decision lines must give, per cell, the counts and the margin actually left (R − observed drain end), never a single PASS that hides a marginal cell. This is also #106's hardware check (repeat resumes after every drop; the per-kind record bound on a long drop storm).
2. **The hooks**, the smallest that work:
   - **m**: today `RESUME_MARGIN_NS` is a `const` (`crates/ezsdr-radio-uhd/src/profile.rs:14`, read at `src/provider/tx.rs:1112`). C7 must set it per Run on the bench build. **It must not become user-visible** (no Spec key, no Binding selector, no env var read in the default build) — e.g. a cargo feature the `hardware` test build enables, or a test-only constructor/setter on the Module behind that feature. If you find it cannot be done without a user-visible surface, stop and report: that is the owner's decision. Note Fable's #106 M3: an environment variable read nowhere on the bench path is useless — prove the hook is read on the exact path the hardware test runs.
   - **Stalls**: a test-side wrapper over `Arc<dyn Device>` (`src/lib.rs:63`, `src/authority.rs:351`) that holds a chosen `tx_send` (the first, for (a); one mid-play, for (b)) for a chosen time before handing it on, as `tests/fake.rs`'s `Descheduled` (line ~3769) does for the FakeDevice. Reuse or generalise `Descheduled` rather than writing a second copy if it fits (AGENTS §9 maintainability 1). A wrapper in test code is not a production hook.
3. **C2 at 100 and 200 MS/s** (`bench/2026-11-18-mt3/README.md` table row C2; `plan/phase8/bench.md` C2 row and section): the tool assumed the host hands the late burst's last chunk before its own time; at these rates it was handed 0.9–1.5 ms after (20/20 INCONCLUSIVE). Read C2's purpose (the late-device-burst rule at high rates: a device burst whose first packet is late is dropped whole) and propose a trial whose premise holds at 100/200 MS/s with the same verdict meaning (e.g. a burst short enough, or booked far enough ahead, that its whole payload is handed before its time; or a different observable), or show with evidence that it cannot be asked without new production surface, and stop on it. If C7's τ is what C2 needs, say how C2 uses it.

## Phase A: the test plan (AGENTS §9 step 2)
Read first: `AGENTS.md` §9 (steps 1–4, "Hardware tests and bench values", Maintainability), the addendum and decisions above, `plan/phase8/reviews/n106-test-plan.md` and `n106-dispositions.md`, `plan/phase8/bench.md` (C1–C7, the `hw` container recipe), `crates/ezsdr-radio-uhd/tests/common/{mt3,mt3_cascade,mt3_tail,mt3_two,tone,run,rig}.rs`, `tests/hardware.rs` (the `hw_mt3_c*` tests), `tests/fake.rs` (`Descheduled`, the `mt3_c*_rehearsal_*` tests, the repeat-resume tests), `src/provider/tx.rs` around the resume (R, `busy`).

The plan lists, for each sequence below (add any the design needs), the test, what it asserts, how it observes it, and the wrong behaviour that would fail it, plus the mutation rows (Rust token anchors; exact substrings for other files) and the killer test of each:
- **S1 hook default.** A build without the bench feature uses 10 ms; nothing a user writes (Spec, Binding, env) changes m. → a test pinning the default path and one pinning that the hook is what the C7 run reads.
- **S2 start forced late, m = 10 ms.** The first send is held past its time → every part of that device burst is dropped; the repeat sends nothing until R = `repeat_boundary(origin, N, max(busy + m, now + lead, front))` and resumes there at phase 0 on its lattice; one `TIME_ERROR { late_at_device }` per incident with `resumed_at = R`. The FakeDevice rehearsal books R's boundary on both sides (a held time that leaves R on one lattice point vs the next).
- **S3 starved mid-play.** A mid-play `tx_send` held for X ms → `TX_UNDERFLOW { starved }` with `resumed_at = R`; silence `[from, R)`; tone back at R phase-continuous. Rehearsal on both sides of the same boundary.
- **S4 m small (0.2 ms) on the fake**: the fake has τ = 0 and a drain pad `max(g, ⌈S/100⌉)` (addendum §2.4), so state what the fake can and cannot show for an early R (a resumed part dropped again → another resume, no cap) and what only hardware shows.
- **S5 verdict functions** (pure, hand-built evidence): clean / dropped-again / silence violated / phase discontinuous / no resume / INCONCLUSIVE (missing capture, overflow on RX, wrong event count) — each line the decision prints, and which one a false PASS or false FAIL on an X310 would come from.
- **S6 τ estimate**: its formula, inputs, resolution at each rate, and a pure test.
- **S7 C2 redesign**: the premise check the trial makes before it judges (INCONCLUSIVE when the premise fails, never PASS), its rehearsal at both sides.
- Per cell, the RF and wiring needs (rates, tone, gain, att ≥ 30 dB in the loopback: the owner confirms before any transmit), the run time per board, and the `bench.md` text (commands, grep lines, what to return) to be written in phase B.
Mark every hardware/UHD behaviour the plan relies on VERIFIED (with source: UHD source, a bench record path) or INFERRED.

## Process
- In phase A you may use DeepSeek for reading/sweeps if it saves you time, but the plan's design decisions are yours. Tasks `ds-<n>-task.md`, logs `ds-<n>.log` in `~/.cache/ezsdr-fixes/2026-11-19-bt2/`. A task file must not start with `---`. Launch: `cd <main tree> && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`; real PID via `pgrep -n -f 'deepseek-v4.1-flash#max'`, check `/proc/<PID>/cwd`. **Wait with the Bash tool's `timeout` set to 600000**: `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`. Every task tells DeepSeek: its shell cuts commands at 120 s; never `mutate.py`, never a mutant or wrong change in the tree; no commit; no files outside the task; phase A changes no repository file.
- Adversarial review of your own plan before reporting: `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`, with special attention to a false PASS or false FAIL each verdict could report on an X310, and to a hook that the bench path does not actually read.

## Report
Final message: the plan's path; a one-paragraph summary of the hook design (and the alternatives you rejected, one line each); the sequences → tests table in short; open questions for the orchestrator (each with your recommendation); anything that would need an owner decision (user-visible surface, a measured value adopted); INFERRED items.


## brief-v2

# bt2 phase A2: revise the test plan after Sol's review — Sonnet drives Go DeepSeek

Same step, tree, branch and rules as `~/.cache/ezsdr-fixes/2026-11-19-bt2/brief.md` (read it; its Process section applies). Still **phase A: no repository change.**

Inputs: your plan `test-plan.md`, Sol's review `sol-plan.md`, and the orchestrator's dispositions `decisions.md` (all in `~/.cache/ezsdr-fixes/2026-11-19-bt2/`). The dispositions are binding: **C2 leaves bt2** (remove S7 and C2's rows and texts from the plan; phase B only adds one bench.md line saying C2's 100/200 MS/s stays out and INFERRED until revisited after C7), Q2 adds 10 MS/s on the bench, and F1–F6, F9, F10 are adopted as written there.

Write `test-plan-v2.md` (the whole plan, not a diff) and a short `plan-v2-changes.md` mapping each adopted finding to the plan text that closes it. For each adopted major, give the failing scenario from Sol's text as a test that would fail on a wrong tool (pseudocode is enough) and say how the corrected verdict reacts. Before reporting, run your adversarial self-review (`~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`) on v2.

Report (Japanese, short): the paths; per finding one line (closed how); new open questions with your recommendation; INFERRED items that changed.


## brief-v3

# bt2 phase A3: revise the test plan after the re-checks — Sonnet

Same step, tree, branch and rules as `~/.cache/ezsdr-fixes/2026-11-19-bt2/brief.md`. Still **phase A: no repository change.**

Inputs (same folder): `test-plan-v2.md`, the re-checks `opus-recheck-plan.md` and `terra-recheck-plan.md`, and `decisions.md` — its last section is binding. Its main change is a simplification: **the tool never attributes a retry to m**; it reports counts and the raw drain evidence, and gains `UNSETTLED(k)`. Remove what that makes obsolete (classes, thresholds, fixtures, rows) rather than keeping it beside the new text.

Write `test-plan-v3.md` (whole plan) and `plan-v3-changes.md` (each finding → the closing text; for each major, the failing scenario as a fixture and the corrected verdict). Re-derive by hand any number you change, against the code. Run your adversarial self-review (`~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`) on v3, and re-read the whole of v3 once at the end for leftovers from v2.

Report (Japanese, short): the paths; per finding one line; open questions with your recommendation.


## brief-b1

# bt2 phase B, sub-step B1: the resume-margin hook — Sonnet drives Go DeepSeek

Main tree `<main tree>`, branch `work/bt2` (main 715ebd67). Work only there; never touch `<worktree>`. Do not commit, stash, push, reset or switch branch. Process, DeepSeek launch, 600000-ms waits, rows via `simq` + `remote_mutate.py`, and the rules of `~/.cache/ezsdr-fixes/2026-11-19-bt2/brief.md` §Process apply (DeepSeek may now edit the tree; no mutant in the tree ever).

The plan: `test-plan-v3.md` in that folder, amended by `decisions.md` (all sections binding; the last says phase B runs in sub-steps B1 → B2 → B3 and lists what each carries). **This run is B1 only:** `UhdRadio::with_measurement_resume_margin(ns)` (Rust-only builder, no feature, no Spec/Binding/env surface), the path to `tx.rs:1112`, tests S1a–e, their rows (C7-01…, token anchors; C7-02 uses `crate::profile::RESUME_MARGIN_NS`), and the design/18 sentences the plan lists for the builder (446–448, UR-22). Nothing of B2/B3. A hand-computed expectation that disagrees with a run: stop and report, never tune.

Exit checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, UHD-feature clippy and the `hardware` build via `plan/maintenance/tools/remote/rdev.py`, `cargo test --workspace`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 715ebd67`; rows run and each dies by assertion. Adversarial review `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.

Report (Japanese, short): `git diff --stat`; tests and rows with outcomes; checks; deviations; INFERRED.


## brief-b2

# bt2 phase B, sub-step B2: the stall hold, the trial body, one-incident rehearsals — Sonnet drives Go DeepSeek

Main tree `<main tree>`, branch `work/bt2` at dab47cc0 (B1, committed). Same rules and process as `brief-b1.md` / `brief.md` §Process in `~/.cache/ezsdr-fixes/2026-11-19-bt2/` (DeepSeek may edit the tree; never a mutant in the tree; rows via `simq` + `remote_mutate.py`; **never `chmod -R`** — chmod 644 named files only). Use DeepSeek for the mechanical parts (tests from your specs, rows, bench.md text) so your own context stays for design and review.

Binding inputs: `test-plan-v3.md` and every section of `decisions.md` (the last two cover B1's review and the F13 refutation).

**B2 scope, in order:**
1. B1's review fixes (decisions.md "B1 review"): the exact builder-to-Core test and its clamp row; S1b's name/doc; the `c7_rehearsal_*` tests moved to a within-Session oracle (after item 2 gives you the probe rows on one axis).
2. `ProbeDevice` hold: send classes Flush / Close / Start / Continuation as distinct inputs (decisions "F9"); a `host_entered` stamp taken **before** the sleep and a return stamp after, both on the probe's own axis (`host_ns`, `probe_device.rs:128`) — every value later mapped through `clock_anchor()` is stamped on the anchor's axis, with a test that the conversion holds on the manual clock (decisions "Terra v3 2").
3. The trial body `common::c7` (Session, control repeat, PN waveform, RX capture, the probe rows, `expected_r` from the rows), shared by the hardware tests and the rehearsals; the hardware `#[ignore]` tests' shells may exist but their verdict is B3.
4. One-incident rehearsals S2 (start forced late) and S3 (starved mid-play), each booking R's boundary on both sides; the run-revealed findings as fixtures written failing-first: the starvation evidence window from `host_entered` (an Underflow tick between entry and return → the trial's evidence holds).
Nothing of the verdict table, S4 or S8 (that is B3).

**For B3, put in your report** the verdict as a grid — resume rows {0, 1, ≥2} × final accepted Start {none, heard, silent} — one outcome per cell, what each needs from the evidence, and the fixture for each (decisions "Terra v3 3, 4, 5 + Opus v3 3, 4, 8": judge only the final accepted Start; a silent final Start counts as dropped only with a TimeError at tick ≥ its time, else NoResume; INFERRED premises print as data, not gates). The orchestrator approves it before B3.

A hand-computed expectation that disagrees with a run: stop and report, never tune. Exit checks as in brief-b1.md, `rules.py --base 715ebd67`; new timing-dependent tests ×20, with all cores busy.

Report (Japanese, short): `git diff --stat`; tests and rows with outcomes; checks; the B3 grid; deviations; INFERRED.


## brief-b3

# bt2 phase B, sub-step B3: B2's review fixes, then the verdict, S4, S8, the hardware tests and bench.md — Sonnet drives Go DeepSeek

Main tree `<main tree>`, branch `work/bt2` at b84acf11 (B1, B2 and a plan-50 docs commit, all committed). Same rules and process as `brief-b1.md` / `brief.md` §Process in `~/.cache/ezsdr-fixes/2026-11-19-bt2/` (DeepSeek may edit the tree; never a mutant in the tree; rows via `simq` + `remote_mutate.py`; never `chmod -R`). Use DeepSeek for mechanical parts (tests from your specs, rows, bench.md text) so your context stays for design and review.

Binding: `test-plan-v3.md` and **every section of `decisions.md`**; the last two are new: "B2 report — B3 grid approved with changes" (the verdict grid, as amended by the orchestrator — implement that table, one fixture per cell, and the three notes under it) and "B2 review … fixed first in B3".

Order:
1. B2's review fixes (decisions "B2 review"), each with its test and row, before any verdict code.
2. The verdict as the approved grid (pure function over the trial's evidence; every cell reachable by a fixture; INFERRED premises printed as data, not gates; the starvation evidence gates on gain only).
3. S4 (fake: Tx-rig exact pins; Session-level only within one Session — cross-Session repeatability is refuted, decisions "B1 report notes"; Session B expects UNSETTLED with rows 1–2 pinned, then per-row `expected_r`), S8 (storm: record bound, RF silence over a 600 ms window on hardware, events ⊆ rows and ≤ resumes; no capture on the fake).
4. The hardware `#[ignore]` tests `hw_c7_late_start`, `hw_c7_starved`, `hw_c7_storm` (cells: m 10/1/0.2 ms × 1 and 10 MS/s, trials 10/20/20 per decisions Q6; ≤ 15 min each per board), their decision lines (per cell: counts of CLEAN / RETRIED / UNSETTLED / INCONCLUSIVE by reason / FAIL by name, the raw drain distances, `delta_first` as signed data), and the FakeDevice rehearsals booking each boundary both sides (AGENTS §9 hardware-test rule).
5. bench.md: a C7 section (commands in the `hw` container recipe, grep lines, what to return; PN transmit — the owner confirms the attenuators and the PN bandwidth ±0.5 / ±5 MHz before any transmit; both `expired` and `late_at_device` named for mode (a)'s first incident); C2's row and section: 100/200 MS/s stays out, the late-device-burst rule at those rates is INFERRED, to be revisited after C7, and C7's `delta_first` is neither a delivery delay nor a bound; the C2 deferral was decided by the orchestrator (correct the "owner 2026-11-19" attribution at bench.md:1584 and :1626).

A hand-computed expectation that disagrees with a run: stop and report, never tune. Exit checks as in brief-b1.md, `rules.py --base 715ebd67`, new timing-dependent tests ×20 with all cores busy, and the `hardware` build.

Report (Japanese, short): `git diff --stat`; per item the tests and rows with outcomes; checks; deviations; INFERRED; the run time of each hardware test per board (estimate).


## brief-b3fix

# bt2 B3 fix round (after the Opus + Terra review of 1124a724) — Sonnet drives Go DeepSeek

Main tree `<main tree>`, branch `work/bt2` at 1124a724 (committed). Same rules and process as `brief-b3.md` / `brief.md` §Process in `~/.cache/ezsdr-fixes/2026-11-19-bt2/`. A full mutation run of 1124a724 may hold the sim queue (`full.log`); wait for it through `simq`.

Fix exactly the adopted findings in the last section of `decisions.md` (reviews `opus-b3.md`, `terra-b3.md`). The main one: the hardware PN period ≥ 1 ms so the R oracle discriminates on an X310, with an explicit `R unresolved` datum when the measured uncertainty exceeds half a period. Every new or retargeted row dies by assertion; a hand-computed expectation that disagrees with a run stops you (report, never tune). Exit checks as in brief-b3.md, plus S2″ (lead 11 ms) and every changed rehearsal ×20 with all cores busy, and an estimate of each hardware test's run time per board with the longer PN (≤ 15 min each).

Report (Japanese, short): `git diff --stat`; per finding the fix, test, row and outcome; checks; deviations; INFERRED.


## brief-final-fix

# bt2 fix round after Sol's code review — Sonnet drives Go DeepSeek

Main tree `<main tree>`, branch `work/bt2` at 500726dd (committed). Same rules and process as `brief-b3.md` / `brief.md` §Process in `~/.cache/ezsdr-fixes/2026-11-19-bt2/`.

Fix exactly the adopted findings in the last section of `decisions.md` (review `sol-code-final.md`: F1–F10). F8 is bench safety: no copyable block may transmit more than one invocation, and each has its board's attenuator/PN confirmation right above it. Write each failing scenario Sol gives as a fixture first, then the fix. Prefer making the tool smaller where a finding allows it (F10). Every new or retargeted row dies by assertion; a hand-computed expectation that disagrees with a run stops you (report, never tune). Exit checks as in brief-b3.md (`rules.py --base 715ebd67`), the changed rehearsals ×20 with all cores busy, the hardware build.

Report (Japanese, short): `git diff --stat`; per finding the fix, fixture, row and outcome; checks; deviations; INFERRED.


## brief-final-fix2

# bt2 fix round 2 (after the side-by-side re-check) — Sonnet, may drive Go DeepSeek

Main tree `<main tree>`, branch `work/bt2` at `d26e8c14` (your fix round, committed). Same rules and process as `brief-final-fix.md` / `brief-b3.md` / `brief.md` §Process in `~/.cache/ezsdr-fixes/2026-11-19-bt2/`. Do not commit, stash, push, reset or switch branch.

Read both re-checks: `terra-recheck-final.md` (FAIL: majors 1–4, minors 5–6) and `opus-recheck-final.md` (PASS: minors N1–N3, N7; notes N4–N6). Dispositions (orchestrator) — fix exactly these, fixture first where a scenario is given:

1. **Clock bracket (Terra 1 = Opus N1): adopted.** The per-send stamp error uses a defensible bound: the Authority's bracket of the whole probe call (`src/authority.rs:99`), not the probe's inner `time_now` width, and never a read that completed after the send returned (no look-ahead past the return; account for the Authority's monotonic clamp, `authority.rs:140`). A fixture with Terra's sequence (a wide accepted read in effect at the held return, a narrow read just after it) that gave `ResumeOffRule` before and the right verdict after.
2. **Unresolved R (Terra 2): adopted, bench safety (a false CLEAN).** When R is unresolved, `r_check` accepts only lattice points within the computed `[lo, hi]`; a point outside is `ResumeOffRule`. Fix the fixture at `c7.rs:937` that expects acceptance one period outside; add Terra's sequence (bounds 150–152 ms, Module resumes at 200 ms on the lattice → not CLEAN).
3. **Storm rows (Terra 3): adopted.** Each retained storm row's R is checked against the rule range from its own incident (the same `expected_r` / `r_check` as a trial row), not only placed between adjacent accepted Starts. Fixture: a retained row's R replaced by another value inside its Start interval but off its rule range → not CLEAN. If a row's own incident lacks the data the rule needs (the sparse event Terra mentions), that row is INCONCLUSIVE, never passed on ordering alone.
4. **Load flake (Terra 4, Opus's flake analysis, N3): adopted, gate trust.** The rehearsals' capture feed is a `drop_oldest` link of 64 blocks (`tests/common/mod.rs:297, 324`); under load the capture thread is starved and the capture gets a hole. (a) Give the C7 capture link enough capacity that a starved capture thread does not evict at 10 MS/s in the rehearsals (choose it from the block size and a stated stall, e.g. ≥ 200 ms at 10 MS/s; say the arithmetic). Check the bench profile's capacity too: if the same link is used on the bench, raise it there as well only if the memory is reasonable, else leave it and rely on (b). (b) The verdict reads the capture link's drop count from the Manifest (`ezsdr-kernel/src/coordinator/ending.rs:119-121, 421`) and, when non-zero, reports its own INCONCLUSIVE reason ("the capture link dropped N blocks") before any hole message. Fixture for (b). Then rerun the c7_/probe_ fake tests ×40 under the same 16-`yes` load as before and report the count; keep the failing runs' full output if any.
5. **Hardware-period rehearsals' exact-R assertions (Opus N7): adopted.** Replace `!terms.unresolved` / `allowed == 1` (`fake.rs:2900-2903, 2951-2954`) with the verdict and "R within [lo, hi]" (from item 2), so a wide read under load cannot fail a correct Module.
6. **Storm continuity (Terra 5 = Opus N2): adopted as code.** Check the storm capture's continuity from its own first sample to the scheduled end, as `bench.md:1798` says; fixture: a hole between the control and the held Start → INCONCLUSIVE.
7. **Silent final part (Terra 6): adopted.** `read_part`'s coverage check (`c7_rf.rs:292`) matches what it reads: a capture with the promised periods for a silent part gets the silent-part verdict, not a coverage error. Fixture at that boundary.
8. **Notes N4–N6: adopted.** N4: a fixture where the `4·p_q` term binds, and a row for it. N5: the two wordings (`MAX_FOLLOWED_STARTS` = the first Start, the resume and 7 retries, in `c7.rs:35` and `bench.md:1711`; `r_check`'s doc matches the whole-width test). N6: correct the comment at `fake.rs:2941-2945` to say what the assertion checks.

Rows: add or retarget rows for 1–8 (each dies by assertion; none by timeout); run the touched and new rows through `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:2,sim03:2 <ids>` (another lane also queues on simq; that is fine). Log `rows-fix2.log` in the work dir. A hand-computed expectation that disagrees with a run stops you (report, never tune).

Exit checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `python3 plan/maintenance/tools/rules.py --base 715ebd67`, the UHD hardware build (`rdev.py` as before), the loaded ×40 of item 4. Remove your load script and every `yes` process (by PID) when done. Test temp files on /tmp.

Report (Japanese, short): `git diff --stat`; per item the fix, fixture, row and outcome; the ×40 count; the capacity arithmetic; deviations; INFERRED.


## brief-final-fix3

# bt2 fix round 3 (minors only) — Sonnet, implement directly

Main tree `<main tree>`, branch `work/bt2` at `f69e94f6` (your fix round 2, committed). Same rules as `brief-final-fix2.md`. Do not commit, stash, push, reset or switch branch.

Re-checks of round 2: `terra-recheck-fix2.md` (FAIL: majors 1–2, the clock bracket) and `opus-recheck-fix2.md` (PASS: minors 1–3, notes). Dispositions (orchestrator):

1. **Terra 1–2 = Opus minor 1 (the recorded bracket is still inside the Authority's; a read installed between the inner send return and the Module's stamp): downgraded to minor, answered in the procedure, no more oracle code.** Both reviewers agree the only outcome is a loud `ResumeOffRule` on a correct Module, never a false CLEAN, and only when the clock thread is preempted in a tiny window next to a send near a lattice boundary; the fake cannot show it. In `plan/phase8/bench.md` C7: (a) correct line ~1710 — the probe's bracket is the closest bound inside the Authority's, not the same (INFERRED: the remaining part is a preemption window); (b) one rule: a `ResumeOffRule` trial whose observed R is exactly one lattice point outside `[lo, hi]` is rerun once before it counts; if the rerun is clean the cell records "one ResumeOffRule at the lattice edge, rerun clean" and is not a FAIL. If the C7 cell summary or the extraction in bench.md needs one printed field to apply this (the distance of the observed R from `[lo, hi]` in lattice points), add it to the trial line and its fixture, with a row.
2. **Opus minor 2 (1024 blocks not checked in a real Session): adopted.** Assert the Manifest's (or the compiled plan's) capture feed capacity = 1024 in one C7 rehearsal, and a row on `c7.rs:679` killed by it.
3. **Opus minor 3 (bench.md:1785 calls the link-drop reason the first): adopted** — say it comes before any hole message.
4. **Notes: the redundant `R ∈ lo..=hi` asserts at `fake.rs:2901` and `fake.rs:2951`** (already enforced by `check().unwrap()`): remove them.

Rows: run the new and touched rows through `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:2,sim03:2 <ids>`; append to `rows-fix3.log` in the work dir. Exit checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p ezsdr-radio-uhd --test fake -- c7_ probe_` once, `python3 plan/maintenance/tools/rules.py --base 715ebd67`. No load run.

Report (Japanese, short): `git diff --stat`; each item; row verdicts.
