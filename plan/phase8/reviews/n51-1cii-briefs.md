# Note 51 stage 1c-ii — briefs (2026-11-14)

## brief-table.md

## Brief — note 51 stage 1c-ii — classification table (plan only)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4` on `main` (1750373: stage 3 landed). This run writes the **classification table and plan only**, to `/home/komatsu/.cache/ezsdr-fixes/2026-11-14-n51-1cii/table.md`; no edits in the tree. Probes only in a copy outside it (AGENTS §7: `rsync -a --no-times --exclude .cargo --exclude target --exclude v3 <tree>/ /tmp/n51-1cii-probe/`, `find /tmp/n51-1cii-probe -type f -exec touch {} +`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`), deleted at the end; or `python3 plan/maintenance/tools/remote/rdev.py -- <cmd>`.

### The stage (plan/phase8/51-test-timing.md §7.2 row 4, Q6 at ~:125 and ~:514)
The ~75 `Frozen` / `released` uses in crates/ezsdr-radio-uhd (lib tests and tests/): start from a classification table; replace with the participant guard (host.rs M1–M7; the 1c-i shape that replaced the 15 `drop(frozen)` sites in stage 1) where it fits; keep `Frozen` where time must stay stopped while the test waits for an owner's reaction (the guard cannot do that: a non-clock wait under it hangs); remove `Frozen` only if nothing needs it.

### The table must give, per site (path:line, test name)
What the freeze protects (which wait, which reaction), whether the wait under it is a clock wait or a non-clock wait (channel, condvar, join…), the treatment (guard / keep Frozen / delete as redundant / other) with the reason, the mutation rows whose filter selects the test (grep the lists) and whether their kill could change, and the risk (a hang under the guard, a vacuous pass). Then: the count per treatment; whether `Frozen` can be removed; sub-steps of ≤ ~1 500 lines that each leave the tree green; the exit checks per sub-step (the moved tests 20 serial + 3 × 8 parallel via rdev; the rows through remote_mutate.py, sim02:7,sim03:7); what you could not decide, with a recommendation. Name any site where the plan's rule does not fit.

Read first: AGENTS.md §7, §9; plan/phase8/51-test-timing.md (§4.4, §7.2, Q6, the 1c records), plan/phase8/51-tables.md, plan/phase8/reviews/n51-1c-* (if present) and n51-3-dispositions.md; crates/ezsdr-radio-uhd/src/host.rs, src/provider/test_support.rs, tests/common/{mod.rs,run.rs}.

Offloading to DeepSeek (reading only, here): Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — here, listing every `Frozen`/`released` site with path:line and the enclosing test — and keep your own effort for the classification. It makes mistakes (misreads concurrency and fake/hardware boundaries): check what it returns against the files. Foreground, one at a time; list what you offloaded.

Report: the table's path, the counts per treatment, open questions with your recommendation, anything INFERRED.

## brief-4a.md

## Brief — note 51 stage 1c-ii, sub-step 4a (the participant guard replaces Frozen inside test_support.rs)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n51-1cii` (from main 1750373). You edit the tree; you do **not** commit, stash, push or switch branches. Risky (it changes the test clock's ownership semantics for 350 lib tests); the plan was reviewed by Sol (PASS). No self-review round: a fresh Opus reviewer reads the diff after you; Sol reviews the whole stage at its end.

### The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-14-n51-1cii/table.md` (§1–§9, appendices) — this run does **sub-step 4a only** (the semantic swap in `crates/ezsdr-radio-uhd/src/provider/test_support.rs`, its helper test, rows N51D-01 replaced, N51D-02 obsolete, E2A2A-10 as the table says). Decisions: `decisions.md` beside it (Q1–Q7 settled; helper name `clock_guard`). Do not do 4b–4d. Background: AGENTS.md §7, §9; plan/phase8/51-test-timing.md §4.4, §7.2 row 4, Q6; crates/ezsdr-radio-uhd/src/host.rs M1–M7.

### Exit checks (table §8, row 4a)
- `cargo fmt --all`, `cargo clippy --workspace --all-targets`, `cargo test -p ezsdr-radio-uhd`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 1750373` (long; foreground).
- The lib binary 20 serial + 3 × 8 parallel via `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>`, 0 failures.
- Mutation: the rows table §8 names for 4a through `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>`: all `killed`; report any change.

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
`git diff --stat`, each check's counts, the rows' outcomes, any deviation from the table and why, what you offloaded, anything INFERRED.

## brief-4bc.md

## Brief — note 51 stage 1c-ii, sub-steps 4b and 4c as one sub-step "4bc" (callers convert to clock_guard)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n51-1cii` (from main 1750373; 4a committed). You edit the tree; you do **not** commit, stash, push or switch branches. Risky (it changes the test clock's ownership semantics for 350 lib tests); the plan was reviewed by Sol (PASS). No self-review round: a fresh Opus reviewer reads the diff after you; Sol reviews the whole stage at its end.

### The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-14-n51-1cii/table.md` (§1–§9, appendices) — this run does **sub-steps 4b and 4c** (every caller in `core/t3.rs`, `core.rs`, `rx.rs`, `mod.rs`, `control.rs`, `tx.rs`, `tx/tests/*`, `c2.rs:581`, `tx.rs:1852` converts to `clock_guard`: `released` inlined, `handed_over` deleted, the W1 drops kept, the N drops deleted; N51D-03 retargeted as the table says). Decisions: `decisions.md` beside it (Q1–Q7 settled; helper name `clock_guard`). Do not do 4d (deleting Frozen/released/handed_over and the texts). Also, from the 4a review: add `assert!(!next_lock_armed())` after the inlined work and before the post-operation `seam_line` at `tx/tests/e3b.rs:~877` (and wherever else table.md §5 risk (a) says a hook is armed inside inlined work); P1302 joins the row run (Sol). Background: AGENTS.md §7, §9; plan/phase8/51-test-timing.md §4.4, §7.2 row 4, Q6; crates/ezsdr-radio-uhd/src/host.rs M1–M7.

### Exit checks (table §8, rows 4b and 4c)
- `cargo fmt --all`, `cargo clippy --workspace --all-targets`, `cargo test -p ezsdr-radio-uhd`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 1750373` (long; foreground).
- The lib binary 20 serial + 3 × 8 parallel (the converted tests) via `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>`, 0 failures.
- Mutation: the rows table §8 / Appendix B names for 4b and 4c, plus P1302 through `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>`: all `killed`; report any change.

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
`git diff --stat`, each check's counts, the rows' outcomes, any deviation from the table and why, what you offloaded, anything INFERRED.

## brief-4d.md

## Brief — note 51 stage 1c-ii, sub-step 4d (delete Frozen, released, handed_over; texts; rows)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n51-1cii` (from main 1750373; 4a and 4bc committed). You edit the tree; you do **not** commit, stash, push or switch branches. A fresh Opus reviewer reads the diff after you; Sol reviews the whole stage at its end.

### The plan
`/home/komatsu/.cache/ezsdr-fixes/2026-11-14-n51-1cii/table.md` (§8 row 4d, §10) and `decisions.md` beside it. This run does **sub-step 4d**:
- Delete `Frozen`, `Frozen::of`/`on`, `released`, `handed_over` from `crates/ezsdr-radio-uhd/src/provider/test_support.rs` (clippy's four dead-code warnings go with them).
- Retire or retarget the mutation rows the table names for 4d.
- Texts: table §10's grep for frozen/released/hand-over wording in the lib tests' docs and comments; `plan/phase8/51-test-timing.md` — Q6 at ~:125 and ~:514: strike the keep-case and state W1 as "drop the guard where the Frozen was dropped" (decision Q2), the N51D-01/-02 and hand-over text at ~:554, §7.2 row 4 done; `plan/phase8/51-tables.md`; `crates/ezsdr-radio-uhd/tests/README.md` if it mentions them.
- From the 4bc review: the stale texts at `tx/tests/e2b.rs:86` (shutdown_owner's doc), `tx/tests/n75.rs:2`, `tx/tests/e3.rs:3`, `tx/tests/e1.rs:2`; rename `_driver` to `_guard` at `tx.rs:~1840`; lengthen N51D-03's anchor with the following `std::thread::sleep(` so it stays unique.
Background: AGENTS.md §7, §9; plan/phase8/51-test-timing.md §4.4, §7.2 row 4, Q6; crates/ezsdr-radio-uhd/src/host.rs M1–M7.

### Exit checks
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo test -p ezsdr-radio-uhd`, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 1750373` (long; foreground).
- The lib binary 20 serial + 3 × 8 parallel via `python3 plan/maintenance/tools/remote/rdev.py [--host sim03] -- <cmd>`, 0 failures.
- Mutation: the rows you retarget plus N51D-01 and N51D-03 through `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>`: all `killed`. The full lists run after you, by the orchestrator.

### Rules (every brief)
- Mutation rows run only through `remote_mutate.py`. Never run `mutate.py` locally, never apply a mutant to the tree; experiments in a copy outside the repository.
- Wait in the foreground for every long command; a command left in the background when you report dies with you.
- Test temp files on /tmp. No build output inside the tree. Kill processes by PID, never by a name pattern.
- Files under `~/.cache/ezsdr-fixes/`, `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only, except the note-51 texts named above. A mutation row that must change with your code may be retargeted in `plan/maintenance/tools/**/mutations*.json`; say so in the report.
- Do not delete a test of a reachable sequence without a replacement. If you delete or rename a symbol, grep the mutation lists for rows whose `filter` or `old`/`new` names it, and fix them.
- Do not add tests that judge by wall-clock time. Do not add mechanisms the plan does not name.
- After `cargo fmt`, `rules.py` must still find every mutation row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits, text updates — and keep your own effort for the hard parts. It makes mistakes (adds unasked mechanisms, misreads concurrency): read its diff and run the tests before you accept it. Foreground, one at a time, do not touch the tree meanwhile; list what you offloaded.

### Report (short)
`git diff --stat`, each check's counts, the rows' outcomes, any deviation from the table and why, what you offloaded, anything INFERRED.

