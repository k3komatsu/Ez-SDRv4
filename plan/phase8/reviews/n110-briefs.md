# #110 + #113 — briefs

## brief

# Phase 8 lane 2: #110 + #113 (phase A: test plan) — Sonnet drives Go DeepSeek

Worktree `~/local_work/Ez-SDRv4-n111`, branch `work/n110` (base main `d4f840ff`, which holds #111/#119 and #120/#121). Work only there; never touch `~/local_work/Ez-SDRv4` (another lane). Do not commit, stash, push, reset or switch branch.

**Risk: risky** (#110 changes an identity — when an ActionId is assigned — and a lifecycle record). **This run is phase A: no repository change.** Write `~/.cache/ezsdr-fixes/2026-11-20-n110/test-plan.md` and stop with a report; Sol reviews it once; then the code (no paper re-check).

## Owner decisions (binding, 2026-11-19 「推奨で」)
- **#110** (`gh issue view 110 -R k3komatsu/Ez-SDRv4`): an admitted start-batch Action dropped by an end before its dispatch is recorded in `termination.undelivered` (RS-16 as written); its ActionId is assigned at admission so it can be recorded (orchestrator). Rejected: stating that the pre-start batch is not "admitted" for RS-16.
- **#113** (`gh issue view 113`): a MockRadio segment cut at or before its origin records `ended_at` = its origin (an empty segment), whether or not its clock was registered; RM-25 / MR-25 state it. Rejected: the stop instant (an end before the start); removing the row (loses the attempted start).

## What the plan covers
- #110: where ActionIds are assigned today (start batch, scheduled Actions, Session Actions, Module submissions) and what moves; every reader of an ActionId that could see a different number or order after the change (action log, Manifest, events, Python client, replay/parity fixtures, `undelivered`); the ends that can drop the batch (`pipeline.rs` ~320–337: after `admit_schedule`, after `start_instances`, right after `Running`), and how each records the dropped ids; RS-16's and the Manifest schema's text; whether any user-visible numbering changes (if a fixture's ids change, say which and why the new numbering is right — a user-visible change beyond the decision goes back to the orchestrator).
- #113: MockRadio's `rx_settle` `None =>` arm (`crates/ezsdr-mock-radio/src/lib.rs` ~1279–1292) and the registered-clock path from stage 4b-2d; the UHD Module's matching path (one rule, both Providers — check the UHD side already does origin, or say what it does); RM-25 / MR-25 text.
- For each sequence: the test, what it asserts, how it observes it, the wrong behaviour that fails it; mutation rows (Rust token anchors) dying by assertion; the `design/` text to change (no issue numbers in design/; `rules.py` limits).

## Process
Use DeepSeek for reading sweeps if it saves context (tasks `ds-<n>-task.md`, logs `ds-<n>.log` in `~/.cache/ezsdr-fixes/2026-11-20-n110/`; a task file must not start with `---`; launch `cd ~/local_work/Ez-SDRv4-n111 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`, real PID via `pgrep -n -f 'deepseek-v4.1-flash#max'`, check `/proc/<PID>/cwd`; **wait with the Bash tool's `timeout` set to 600000**: `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`; tell DeepSeek: shell cuts at 120 s, read-only in phase A, no commit). Adversarial self-review of the plan: `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.

## Report
Japanese, short: the plan's path; the ActionId change in one paragraph (what numbering users can see before/after); #113's both-Providers check; open questions with your recommendation; INFERRED items.

## brief-b

# #110 + #113 phase B: plan v2 and the code — Sonnet drives Go DeepSeek

Same worktree, branch and rules as `~/.cache/ezsdr-fixes/2026-11-20-n110/brief.md` (its Process applies; DeepSeek may now edit the tree; never a mutant in the tree; rows via `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` from the worktree, `chmod 644` named files only; tests local).

Binding: `test-plan.md`, `note.md` and every section of `decisions.md` in that folder (the last section is Sol's review `sol-plan.md`, all adopted). First write `test-plan-v2.md` (the whole plan with the dispositions applied; no paper re-check follows), then implement it: #110 (the start batch into `Shared`, ids at admission, atomic batch, the one freeze drains it; tests first where a sequence fails on the current code) and #113 (MockRadio's unregistered receive cut ends at the origin), the `design/` text (RS-16, KC-17, KC-24, RM-25, MR-25 and whatever the plan lists; no issue numbers in design/), the `ActionId` description and the regenerated schema (keep `schema_freeze` green; no version bump, no SCHEMA_CHANGELOG entry).

Every row dies by assertion; a hand-computed expectation that disagrees with a run stops you (report, never tune). Exit checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the Python suite if the client is touched (`PYTHONDONTWRITEBYTECODE=1`, venv `~/.cache/ezsdr-venv`, `EZSDR_SERVER` from this tree), new timing-dependent tests ×20 with all cores busy, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base d4f840ff`, the rows you add or touch. Adversarial self-review `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.

Report (Japanese, short): v2's path; `git diff --stat`; per test and row the outcome; checks; every user-visible change (ids, Manifest) in one line each; deviations; INFERRED.

## brief-fix

# #110 + #113 fix round (after the Opus + Terra review of a53a8917) — Sonnet

Same worktree, branch and rules as `brief.md` / `brief-b.md` in `~/.cache/ezsdr-fixes/2026-11-20-n110/`. HEAD = a53a8917 (committed). A full mutation run of a53a8917 may hold the sim queue; wait through `simq`. Fix exactly the adopted findings in the last section of `decisions.md` (reviews `opus-code.md`, `terra-code.md`) — text, comments, one debug assertion, the `ActionId` description and regenerated schemas, and two rows that must die by assertion. Exit checks as in brief-b.md (`rules.py --base d4f840ff`, `schema_freeze` green). Report (Japanese, short): `git diff --stat`; per finding the fix and row outcome; checks.

## brief-fix2

# #110 + #113 fix round 2 (after Sol's code review) — Sonnet

Same worktree, branch and rules as `brief.md` / `brief-b.md` in `~/.cache/ezsdr-fixes/2026-11-20-n110/`. HEAD = cecf798b (committed). Fix exactly the adopted findings in the last section of `decisions.md` (review `sol-code.md`): S9 observes the second push (or the final effective gain 2.0) and a new row "stop the dispatch loop after the first push" dies by assertion; the complete lock record at the early-end cancel; design/07's citation text. Exit checks as in brief-b.md (`rules.py --base d4f840ff`), the changed tests ×20 with all cores busy. Report (Japanese, short): `git diff --stat`; per finding the fix and row outcome; checks.
