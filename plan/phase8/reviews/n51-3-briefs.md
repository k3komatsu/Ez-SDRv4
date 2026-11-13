# Note 51 stage 3 — briefs (2026-11-13/14)

## brief-plan.md

## Brief — note 51 stage 3 — test plan (plan only)

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-3` at main `e08c8c1` (stage 2c landed). You are the implementer (Claude Code Sonnet subagent). This run writes the **plan only**, to /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-3/test-plan.md; no edits in the tree. Probes only in a copy outside it (`rsync -a --no-times --exclude .cargo --exclude target --exclude v3 <tree>/ /tmp/n51-3-probe/`, then `find /tmp/n51-3-probe -type f -exec touch {} +`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`), deleted at the end. Other lanes build on this host.

### Stage 3 (plan/phase8/51-test-timing.md §7.2 row 3, §8, Q5)
- **S2** — tests on a real-clock FakeDevice (`ClockMode::Real`: owner delay becomes device delay). The gate marks them `real-clock (unclassified)` (the latest list: `real-clock (unclassified)` in ~/.cache/ezsdr-fixes/gates/n51-2c/gate.json; the classes in note 51 §6.5 and the table at `:385`: `fake_real` and 12 explicit sites, 66 default-Real sites). One by one: move to the manual clock (the mark disappears) or declare the row `"kill": "bound"` with the reason. After stage 3, a marked row without a declaration is broken (`:366`) — plan that enforcement (mutate.py / gate.py) too.
- **S3** — Kernel `crates/ezsdr-kernel/tests/device_paced.rs`, 12 wall-bound tests (ms bounds 4, 1 s bounds 7, "50 blocks in 100 ms" 1; 6 are phase7 filters G05, G08, G36, C05, C06, C07): rewrite by P3 / Q5 (`:512`: the ms bounds and G05 become observations, …) plus the scan the note names.
- Deferred to stage 3 by earlier stages: `rx_start_lead_group_runs_on_the_fake` (its plain failures under injected delay, §3.4 and the 2c record `:574-577`); the `Gated` real-time waits (two tests, `:381`, `:397`, `:411`).

### The plan must give
For each test or site: today's class and evidence (path:line), the chosen treatment (manual clock / observation / bound declaration / rewrite), what it asserts after and how it observes it, the wrong behaviour that would fail it, the mutation rows whose filter or kill changes (Rust token anchors; the existing rows' outcomes), and the hardware audit table for any shared trial whose expectation changes (AGENTS §9). Stage it in sub-steps of ~1 500 lines that each leave the tree green. Name what you could not decide, with a recommendation.

Read first: AGENTS.md §7, §9; plan/phase8/51-test-timing.md (§3.4, §5, §6.5, §7.2, §8, the appendix Q5/Q6, the "段 2c の後" record); plan/phase8/51-tables.md; plan/phase8/reviews/n51m-*, n51-2a-*, n51-2c-*; plan/maintenance/tools/mutate.py, gate.py; crates/ezsdr-radio-uhd/src/host.rs, src/device.rs (ClockMode), tests/common/{mod.rs,run.rs,infra.rs}, crates/ezsdr-kernel/tests/device_paced.rs.

Offloading to DeepSeek (reading only, here): Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet and is fast: you may hand it bulk reading (e.g. listing every Real-clock FakeDevice site with path:line), at your judgment. It makes mistakes: check what it returns against the files. Foreground, one at a time; list what you offloaded.

Report: the plan's path, a short summary, open questions with your recommendation, anything INFERRED.

## brief-plan-v2.md

## Brief — note 51 stage 3 — amend the test plan (plan only) → test-plan-v2.md

You are the implementer (Claude Code Sonnet subagent). This run **amends the plan only**. Write `/home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-3/test-plan-v2.md` (a full plan, not a diff; keep v1 untouched). No edits in the main tree `/home/komatsu/local_work/Ez-SDRv4` (on `main`, 8d96743; read only). Probes, if any, only in a copy outside it (`rsync -a --no-times --exclude .cargo --exclude target --exclude v3 <tree>/ /tmp/n51-3-probe/`, then `find /tmp/n51-3-probe -type f -exec touch {} +`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`), deleted at the end. Other lanes may build on this host; keep local load light (or use `plan/maintenance/tools/remote/rdev.py -- <cmd>`). Never run `mutate.py` locally and never apply a mutant anywhere in the tree.

### Inputs (same folder)
- `brief-plan.md` — the original brief (scope S2, S3, the deferred items; what the plan must give).
- `test-plan.md` — your v1 plan.
- `sol-plan.md` — Sol's plan review: FAIL, M1–M3, m4–m6.
- `plan-decisions.md` — the orchestrator's dispositions: all six findings adopted, plus open questions 1–8 settled.

### The base moved
v1 was written at e08c8c1. Since then main gained 3ef2764 (#67: receive open stops first, cut before a queued start) and 8d96743 (stage-4 bench tools: S2, T200, S5, S4). Both touch `crates/ezsdr-radio-uhd/tests/common/` and `plan/maintenance/tools/mutations.json`. Re-check every `path:line` you cite and the S2 inventory against 8d96743: new tests or rows on a real-clock FakeDevice (marked `real-clock (unclassified)`) must be classified too; say which ones are new.

### What to change
- **M1** (G08, G36 in `crates/ezsdr-kernel/tests/device_paced.rs`, support `tests/support/paced.rs`): a test must fail when the wake is removed. Gate the event's emission on an observed parked `wait_for` (the test sees the waiter parked before it delivers MARK); for G36 also prove the callback runs before the schedule call returns. State how "parked" is observed without a wall-clock oracle, and the mutation that each test kills.
- **M2** (`rx_start_lead_group_runs_on_the_fake`, `tests/common/measurement.rs`): the replacement for the dropped SHORT assertion covers event decoding → trial association → the reported SHORT classification, from synthetic event records (wrong event name, undecodable `requested.ticks`, unassociated trial must each fail it). Name the function boundary it calls.
- **M3** (enforcement): a marked row with no declaration is broken whatever its outcome (killed, survived, equivalent, known survivor); baseline failures stay baseline failures; the cached-baseline path is covered. Checked both in `mutate.py`'s row outcome and in `gate.py`'s accounting. List the tests (tool self-tests) for each case.
- **m4** J58: recorded equivalent (`mutations.json` ~:758-766). Keep it equivalent; classify its filter; no killer search.
- **m5**: the scan and any upper bound you keep must agree (v1 kept a 1 s bound while the scan forbids bounds < 3 s). Pick one rule and apply it to both.
- **m6**: the sleep allow-list is per occurrence (pinned pattern or line-content), not per function; add the bypass Sol gives (`sleep(100 ms); assert!(count(...) >= 50)` inside an approved function) as a scan fixture that must be rejected.
- Fold in the decided open questions (plan-decisions.md 1–8) as decisions, not questions; J58 per m4 replaces decision 4.
- Re-check the sub-step split (§8 of v1, ~1 500 lines each, each green) after these changes.

### Report
The v2 path; a list mapping M1–M3, m4–m6 to the v2 sections that answer them; what changed because of the base move; remaining open questions with a recommendation; anything INFERRED. Keep the report short.

Offloading to DeepSeek (reading only in this run): Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast; you may hand it bulk reading (for example re-listing real-clock FakeDevice sites at 8d96743 with path:line) at your judgment. It makes mistakes: check what it returns against the files. Foreground, one at a time; list what you offloaded.

## brief-3ab.md

## Brief — note 51 stage 3, sub-step 3ab (S2: real-clock FakeDevice tests → manual clock)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n51-3` (from main 8d96743). You edit the tree; you do **not** commit, push or switch branches. Risk: not a risky step (test code only); no self-review round — a fresh Opus reviewer reads the diff after you, then Sol at the end of stage 3.

### The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-3/test-plan-v2.md` — §3 (S2), §3.5, §5.2, §8 rows **3a and 3b** (this run does both), appendices A–C. Decisions: `plan-decisions.md` in the same folder (all of it binds; 1 `FakeConfig::default()` stays Real; 2 the ten probe-tool rehearsals keep Real). Do **not** do 3c–3e (Kernel rewrite, scan, `rx_start_lead`, declarations/enforcement). Background: AGENTS.md §7, §9; plan/phase8/51-test-timing.md §6.5, §7.2.

### Do
- 3a: `test_support.rs` three helpers force Manual; the 14 per-site edits; the six real-by-design tests keep their explicit Real; drop the redundant explicit `Manual` at `core.rs:~1956`.
- 3b: the 13 `fake` tests and 8 `x3xx` tests to Manual; sub-step D of 1c (markers, T10 with a Manual-clock `Gated`, N51C-11) per §5.2.
- If a moved test hangs or fails on Manual and the plan did not foresee it, do not force it: leave it Real, say so in the report with the evidence (it then needs a declaration in 3e).

### Exit checks (§8 rows 3a, 3b)
- `cargo fmt --all`, `cargo clippy --workspace --all-targets`, `cargo test` of the touched crates, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 8d96743` (it takes ~5 min with `--sentences`; run it in the foreground with a long timeout).
- Repetition: the lib binary 20 serial + 3 × 8 parallel; each moved fake/x3xx test 20 serial + 3 × 8 parallel; 0 failures. Run the parallel load on sim02/sim03 through `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>` so this host stays usable for other lanes; single runs may be local.
- Mutation: `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:8,sim03:8 <IDs>` over the rows §8 names for 3a and 3b (the 155 lib ids; the 21 + 2 relieved rows, P245/P442/P467, N51C-11). Expect no `ERROR`; every row that was `killed` stays `killed`. Report the counts and any row whose outcome changed.

### Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree; experiments in a copy outside the repository.
- Wait in the foreground for every long command (tests, remote_mutate, rules.py); a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/` and `plan/`, `handoff.md`, `AGENTS.md` are the orchestrator's: read only.
- Do not delete a test of a reachable sequence without a replacement. If you delete or rename a test or a symbol, grep `plan/maintenance/tools/**/mutations*.json` (and phase lists) for rows whose `filter` names the test or whose `old`/`new` names the symbol, and fix them.
- Do not add tests that depend on wall-clock timing. Do not add mechanisms the plan does not name.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits across files, renames, new tests that follow an existing pattern, mutation rows, text updates — and keep your own effort for design, review and the hard parts. What to hand over is your call. DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries): read its diff and run the tests before you accept it. Run it in the foreground, one at a time, and do not touch the tree meanwhile; list what you offloaded in your report.

### Report (short)
Files changed (`git diff --stat`), what each check gave (counts), tests left Real that the plan moved and why, mutation rows whose outcome changed, what you offloaded, anything INFERRED.

## brief-3c1.md

## Brief — note 51 stage 3, sub-step 3c-1 (Kernel `device_paced.rs` rewrite)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n51-3` (3ab committed as c40a5f5 on 8d96743). You edit the tree; you do **not** commit, stash, push or switch branches. Not a risky step in production terms (test code and test doubles only); no self-review round — a fresh Opus reviewer reads the diff after you, Sol at the end of stage 3.

### The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-3/test-plan-v2.md` §4.1, §4.2, §4.4, §4.5, §4.6 and §8 row **3c-1**. Decisions: `plan-decisions.md` (same folder; all of it binds, including the "Opus re-check of v2" section). Do **not** do 3c-2 (the scan, `KERNEL_WALL_ALLOWED`, fixtures, `governance.rs`, P3a text), 3d or 3e. Background: AGENTS.md §7, §9; plan/phase8/51-test-timing.md §5, Q5.

### Binding points from the re-check (read plan-decisions.md for the full text)
- The G08/G36 `wait_for` runs on a **detached `std::thread::spawn`** worker (as `device_paced.rs:~551`), never scoped or joined: a joined worker turns the kill into a test timeout (ERROR) and breaks decision E.
- **N1**: G36's `fired_before_schedule_returned` counter is asserted **once, `>= rounds`, after `manifest_of(run)`** (data thread joined) — no per-round read.
- The `waiting` predicate in the double replaces the 20 ms window, capped at 10 s (§4.5 item 5).
- Every positive wall expiry you keep is `Wall::from_secs(n)` with n ≥ 3 (decision G); #11's ms bound is dropped; #12 stays a negative wait (pinned in 3c-2).
- No `"kill": "bound"` declarations on Kernel rows (decision E).

### Exit checks (§8 row 3c-1)
- First: the new `woken_rounds` helper 20× on the unmutated tree.
- `cargo fmt --all`, `cargo clippy --workspace --all-targets`, `cargo test -p ezsdr-kernel`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 8d96743` (long; foreground, long timeout).
- The Kernel suite 20× serial and 3 × 8 parallel, 0 failures — the parallel load on sim02/sim03 via `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>`.
- Mutation: `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:7,sim03:7 <the §4.6 ids>`: all `killed` as §4.6 says (G05, G08, G36 must be `killed`, not ERROR). Report each row's outcome and any change from before. (7 shards per host: another lane may need the remaining cores.)

### Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree; experiments in a copy outside the repository.
- Wait in the foreground for every long command; a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/`, `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only. A mutation row that must ship with your code may be added or retargeted in `plan/maintenance/tools/**/mutations*.json`; say so in the report.
- Do not delete a test of a reachable sequence without a replacement. If you delete or rename a test or a symbol, grep the mutation lists for rows whose `filter` names the test or whose `old`/`new` names the symbol, and fix them.
- Do not add tests that judge by wall-clock time (expiry-equals-failure bounds of ≥ 3 s are allowed, as the plan says). Do not add mechanisms the plan does not name.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits across files, renames, new tests that follow an existing pattern, mutation rows, text updates — and keep your own effort for design, review and the hard parts. What to hand over is your call. DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries): read its diff and run the tests before you accept it. Run it in the foreground, one at a time, and do not touch the tree meanwhile; list what you offloaded in your report.

### Report (short)
`git diff --stat`, each check's counts, each §4.6 row's outcome, any test whose treatment differs from §4.1/§4.2 and why, what you offloaded, anything INFERRED.

## brief-3c2.md

## Brief — note 51 stage 3, sub-step 3c-2 (Kernel wall-time scan)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n51-3` (3ab c40a5f5 and 3c-1 c7024a5 committed on 8d96743). You edit the tree; you do **not** commit, stash, push or switch branches. Not a risky step in production terms (test code and test doubles only); no self-review round — a fresh Opus reviewer reads the diff after you, Sol at the end of stage 3.

### The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-3/test-plan-v2.md` §4.3 (R1–R3, `KERNEL_WALL_ALLOWED`, the 10 fixtures, `governance.rs` walking the Kernel file, P3a) and §8 row **3c-2**. Decisions: `plan-decisions.md` (same folder; all of it binds). Do **not** do 3d or 3e. Background: AGENTS.md §7, §9; plan/phase8/51-test-timing.md §5, §8, Q5. Note 51's P3a text and the Q5 correction (§8 row 3c-2) are yours in this sub-step: edit `plan/phase8/51-test-timing.md` only for those.

### Binding points (plan-decisions.md: the "Opus re-check of v2" and "3c-1" sections)
- One rule (decision G / m5): every positive wall expiry is `Wall::from_secs(n)` with n ≥ 3, or a pinned allowance.
- Allowances are per occurrence (m6): `(fn, call tokens, exact count, reason)`; the pin's spelling is the code's (`std::thread::sleep(...)` where the code says so). Sol's bypass (`sleep(100 ms); assert!(count(...) >= 50)` inside an approved function) is a fixture that must be rejected. Every allowance must be used (a stale allowance fails).
- R2's call list includes `wait_parked_until(`. R3 matches any path ending in `sleep(` (so `use std::thread::sleep;` then `sleep(` is caught).
- From the 3c-1 review: fix the header comment of `crates/ezsdr-kernel/tests/device_paced.rs:4-5` to say "no positive verdict is a wall interval … every positive expiry left is at least 3 s" (negative waits are pinned under P3a); and replace the three copies of the park-probe capture (`let mut park = None; paced_with(|a| { park = Some(a.park_probe()); … })`, `device_paced.rs` ~:356, ~:547, ~:880) with one helper (e.g. `paced_parked(adjust) -> (Paced, WallPark)`), behaviour unchanged.

### Exit checks (§8 row 3c-2)
- `cargo fmt --all`, `cargo clippy --workspace --all-targets`, `cargo test -p ezsdr-kernel` (the `governance` binary and `device_paced`), `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 8d96743` (long; foreground).
- The scan's own mutation rows (add them with the code; Rust token anchors) through `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>`: all `killed`. Also re-run G08, G36, G05 and C09 (the tests the helper refactor touches) — expect `killed`.

### Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree; experiments in a copy outside the repository.
- Wait in the foreground for every long command; a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/`, `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only, except `plan/phase8/51-test-timing.md` for P3a and the Q5 correction as above. A mutation row that must ship with your code may be added or retargeted in `plan/maintenance/tools/**/mutations*.json`; say so in the report.
- Do not delete a test of a reachable sequence without a replacement. If you delete or rename a test or a symbol, grep the mutation lists for rows whose `filter` names the test or whose `old`/`new` names the symbol, and fix them.
- Do not add tests that judge by wall-clock time (expiry-equals-failure bounds of ≥ 3 s are allowed, as the plan says). Do not add mechanisms the plan does not name.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits across files, renames, new tests that follow an existing pattern, mutation rows, text updates — and keep your own effort for design, review and the hard parts. What to hand over is your call. DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries): read its diff and run the tests before you accept it. Run it in the foreground, one at a time, and do not touch the tree meanwhile; list what you offloaded in your report.

### Report (short)
`git diff --stat`, each check's counts, each new row's outcome, the allowance table as landed, any deviation from §4.3 and why, what you offloaded, anything INFERRED.

## brief-3d.md

## Brief — note 51 stage 3, sub-step 3d (`rx_start_lead` rehearsal: SHORT judged by a pure test)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n51-3` (3ab c40a5f5, 3c-1 c7024a5, 3c-2 2c13fc3 committed on 8d96743). You edit the tree; you do **not** commit, stash, push or switch branches. Not a risky step in production terms (shared trial code, a pure extraction; test code); no self-review round — a fresh Opus reviewer reads the diff after you, Sol at the end of stage 3.

### The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-3/test-plan-v2.md` §5.1 (M2's shape: extract `restart_stage`, the pure test `a_restart_stage_reports_what_the_events_say` fed synthetic events built from the Provider's own payload types, drop the SHORT assertion from the rehearsal, seven new rows and N47B-19's second row) and §8 row **3d**; §7's hardware audit table for the shared trial code. Decisions: `plan-decisions.md` (same folder; all of it binds). Do **not** do 3e. Background: AGENTS.md §7, §9 ("Hardware tests and bench values").

### Binding points
- From the v2 re-check: `restart_stage` filters `delivered` events itself (`events_of` takes `&Manifest`, `tests/common/mod.rs:~577`); the row on `.min()` needs a longer anchor (`.min()` occurs 6 times in the file; `anchor.py` must find each row exactly once).
- The pure test must fail for: a wrong event name, an undecodable `requested.ticks`, an unassociated trial (origin mismatch), the min-lead choice, and the reported row/classification (M2). The rehearsal keeps its LATE assertion from the real Manifest.
- The shared trial code is used by hardware tests: report AGENTS §9's audit table (test, expectation, still right / fixed / cannot tell) for every hardware test whose expectation goes through `restart_stage`.
- Carried from the 3ab review (same crate), in this sub-step: (1) `rig_with`'s doc in `crates/ezsdr-radio-uhd/src/provider/test_support.rs:~28` says it overrides `config.clock` (manual) and that a real-clock test builds its own device and calls `rig_on`; (2) remove the now-redundant `clock: ClockMode::Manual` at `rig_with` callers (`mod.rs:901`, `core.rs:1921,1942`, `control.rs:1530`, `rx.rs:2733,2786`, `tx.rs:1771,4106,4230,4350,4379,4445,4521,4573,5614,6313`, lines approximate) — pure deletion, check that no mutation row anchors on that text (`rules.py` will tell); (3) `tests/fake.rs:~4075` `bound_expired` doc: say the marker is right because a bound is not an assertion (note 51), not that a held thread "says nothing about the code under test".

### Exit checks (§8 row 3d)
- `cargo fmt --all`, `cargo clippy --workspace --all-targets`, `cargo test -p ezsdr-radio-uhd`, the `hw-build` check (compiles the shared hardware code: `cargo build -p ezsdr-radio-uhd --features uhd --tests` in the dev image via `rdev.py`), `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 8d96743` (long; foreground).
- The rehearsal 20 serial + the 12 injection points (P5' as a script), 12/12 green, on sim02/sim03 via `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>`.
- Mutation: `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` over N47B-19, N47B-36, the seven new rows and any row whose filter selects a test you changed (grep the lists): all `killed`.

### Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree; experiments in a copy outside the repository.
- Wait in the foreground for every long command; a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/`, `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only. A mutation row that must ship with your code may be added or retargeted in `plan/maintenance/tools/**/mutations*.json`; say so in the report.
- Do not delete a test of a reachable sequence without a replacement. If you delete or rename a test or a symbol, grep the mutation lists for rows whose `filter` names the test or whose `old`/`new` names the symbol, and fix them.
- Do not add tests that judge by wall-clock time (expiry-equals-failure bounds of ≥ 3 s are allowed, as the plan says). Do not add mechanisms the plan does not name.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits across files, renames, new tests that follow an existing pattern, mutation rows, text updates — and keep your own effort for design, review and the hard parts. What to hand over is your call. DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries): read its diff and run the tests before you accept it. Run it in the foreground, one at a time, and do not touch the tree meanwhile; list what you offloaded in your report.

### Report (short)
`git diff --stat`, each check's counts, each row's outcome, the hardware audit table, any deviation from §5.1 and why, what you offloaded, anything INFERRED.

## brief-3e.md

## Brief — note 51 stage 3, sub-step 3e (declarations and enforcement)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n51-3` (3ab c40a5f5, 3c-1 c7024a5, 3c-2 2c13fc3, 3d 1bf3d2d committed on 8d96743). You edit the tree; you do **not** commit, stash, push or switch branches. It changes the gate's semantics (what counts as a broken mutation row): Sol reviews the whole stage after you; no self-review round — a fresh Opus reviewer reads the diff after you, Sol at the end of stage 3.

### The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-13-n51-3/test-plan-v2.md` §6 (6.1 `mutate.py`, 6.2 `gate.py`, 6.3 the tool self-tests, 6.4 J58 and the other 26, 6.5 landing order), §3.5 (the 27 rows that stay marked) and §8 row **3e**. Decisions: `plan-decisions.md` (same folder; all of it binds; M3, m4, decision A). Background: AGENTS.md §7, §9; plan/phase8/51-test-timing.md §6.5, §8; stage M's record `plan/phase8/reviews/n51m-*`.

### Binding points
- M3: a row whose run is marked real-clock and that declares no `"kill": "bound"` is broken (`ERROR`, with its reason) **whatever its outcome**: killed, survived, recorded equivalent, known survivor. `BASELINE FAILED` stays a baseline failure. The cached-baseline path carries the mark. Checked in `mutate.py`'s row outcome **and** again in `gate.py`'s accounting (an absent `declared` counts as false; `known-survivors.txt` never excuses it).
- Self-tests: the 8 `mutate.py` cells and 6 `gate.py` cells of §6.3, plus one more (re-check minor): a marked, undeclared row that is already `ERROR` for another reason keeps its own `why`.
- J58 (m4, decision A): stays `equivalent`; gets `"kill": "bound"` as the classification of its filter, with its `equivalent` text kept.
- The other 26 of §3.5 get `"kill": "bound"` with a one-line reason each (why the test stays on the real clock).
- Text: docstrings of `mutate.py`/`gate.py`, `crates/ezsdr-radio-uhd/tests/README.md`, note 51 §6.5 and §8, `plan/phase8/51-tables.md` §6 — and from the 3ab review, `51-tables.md:~343` (T7: "inline … sub-step D deferred") and `:~93` (W14: "markers in sub-step D") are now done (3ab), and "T10" means two things (the Gated fixture test in the plan vs the raw-group deadline at `51-tables.md:~346`): rename one so they differ.
- Notes for the record, no change here: the kc_36 TTL tests can fail under load (m4 of the 3c-2 review) and the scan's pins key on the fn name alone; list both under "open" in your report so Sol sees them.

### Exit checks (§8 row 3e)
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/mutate.py --self-test`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/gate.py --self-test`, `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py --e2e` (if it takes `--e2e` alone; read its help), `rules.py --base 8d96743` (long; foreground), `rules.py --self-test`.
- Do **not** run the gate or the full mutation lists: the orchestrator runs them after you (the gate's expectation: `real-clock (undeclared)` empty, `real-clock (declared)` = 27).

### Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree; experiments in a copy outside the repository.
- Wait in the foreground for every long command; a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/`, `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only, except `plan/phase8/51-test-timing.md` §6.5/§8 and `plan/phase8/51-tables.md` as above. A mutation row that must ship with your code may be added or retargeted in `plan/maintenance/tools/**/mutations*.json`; say so in the report.
- Do not delete a test of a reachable sequence without a replacement. If you delete or rename a test or a symbol, grep the mutation lists for rows whose `filter` names the test or whose `old`/`new` names the symbol, and fix them.
- Do not add tests that judge by wall-clock time (expiry-equals-failure bounds of ≥ 3 s are allowed, as the plan says). Do not add mechanisms the plan does not name.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits across files, renames, new tests that follow an existing pattern, mutation rows, text updates — and keep your own effort for design, review and the hard parts. What to hand over is your call. DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries): read its diff and run the tests before you accept it. Run it in the foreground, one at a time, and do not touch the tree meanwhile; list what you offloaded in your report.

### Report (short)
`git diff --stat`, each check's counts, each self-test's cell count, the 27 declarations with their reasons (a table), any deviation from §6 and why, the open notes, what you offloaded, anything INFERRED.

## Fix rounds sent by message

- 3c-2 fix round: the six adopted findings of the 3c-2 sub-step review (dispositions below).
- 3e fix round: `declared` computed once in `run_one` and pinned by self-test cells.
- Sol code minor: integer-suffixed literals (`1_u64`) stripped before the scan parses them; fixtures and one row.
