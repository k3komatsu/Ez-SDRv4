# #120 + #121 — briefs

## brief

# Phase 8 lane 2: #120 + #121 — widen KC-24a to every coordinator lock (phase A: inventory, design, test plan) — Sonnet drives Go DeepSeek

Worktree `~/local_work/Ez-SDRv4-n111`, branch `work/n120` (base main `cefd067c`, which holds #111/#119). Work only there; never touch `~/local_work/Ez-SDRv4` (another lane). Do not commit, stash, push, reset or switch branch.

**Risk: risky** (a Kernel lock discipline, a lifecycle, deadlocks). **This run is phase A: no repository change.** Write `~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md` and stop with a report; Sol reviews it; phase B (code) is a later run.

## Why
`mech:supplied-callback-under-lock` reached three strikes with #103's F1, #111 and #119; the design note (`plan/phase8/reviews/n111-note.md`) answered with KC-24a's rule — no supplied `TimeAuthority`/`HostClock` call under the **admission or machine** lock — pinned by lock records on the watched doubles and a sweep. #120 and #121 are instances 4 and 5 under *other* locks: `wake_handle` held across the Authority's `schedule` (KC-46a, `coordinator/paced.rs` `wake`), and `links` held across a supplied `DataLink::drops()` (`coordinator/ending.rs` RS-6 step 6; Manifest assembly then waits on `links` forever after RS-8a abandons step 6). Read both issues (`gh issue view 120 -R k3komatsu/Ez-SDRv4`, `121`). The rule was too narrow; the orchestrator's decision is to widen it rather than patch two sites.

## Goal
1. **Inventory** (the evidence for the rule's "every"): every lock the coordinator (`crates/ezsdr-kernel/src/coordinator/`, `run.rs`, and whatever they call while holding a lock) takes, and for each, every call made while it is held into supplied code — `TimeAuthority`, `HostClock`, `DataLink` (publish, drops, …), `Provider`/Module methods, Executor, Sink, registered checks, `coerce`. A table: lock, site (path:line), supplied call, whether a Module/Provider thread can wait on that lock, and the disposition (move the call out: read before / act after, copy out under the lock and call after; or keep it, with the reason — e.g. KC-24 steps 10–12's `coerce` and registered checks by design). State the exact `grep`s you ran so the orchestrator can rerun them.
2. **The widened rule** (KC-24a text, its Ceiling for the kept exceptions, and KC-46a's wake): one sentence a reviewer can check, e.g. "the coordinator calls no supplied object's method while it holds a coordinator lock a supplied object's thread can wait on, except …". Keep it one rule; no new mechanism unless a site cannot be fixed by read-before / act-after / copy-out.
3. **#120**: the wake design without `wake_handle` held across `schedule` (and without losing a pending wake's handle from the freeze's drain — the issue's two candidates: a list of handles, or …). Note #111's freeze gate and RS-6/RS-7/KC-46a texts just landed (cefd067c); the late-handle rule there (a handle published after the data thread's step 1 is cancelled by the cleanup's later freeze, after the stops; such callbacks call no Provider) must still hold.
4. **#121**: `drops()` called with `links` released (copy out), and Manifest assembly never waiting forever on a lock a wedged supplied call holds.
5. **Pinning**: extend the existing lock records (`lock_record`, `Serial<T>`, the watched doubles) so every new lock in the rule is recorded, and extend the sweep (T11–T14 in `tests/coordinator.rs` / `tests/device_paced.rs`, `support/start_window.rs`) so a supplied call under *any* recorded lock fails it — a sixth instance fails the sweep without a new test. A watched `DataLink` double for #121's path.

## Test plan (AGENTS §9 step 2)
For each sequence — #120's two shapes (a worker holding what an ordinary wake's `schedule` needs calls `disconnect_signal`; an ordinary wake's handle overwritten by the unconditional wake before the freeze), #121's wedged `drops()` then Manifest assembly, the sweep over a Simulation and a device-paced Run, and any site the inventory moves — the test, what it asserts, how it observes it (lock records, hooks; never a wall-clock oracle; a timeout only as the infra backstop), and the wrong behaviour that fails it; mutation rows (Rust token anchors) each dying by assertion. Spec text to change (design/06 KC-24a, KC-46a, RS-6 step 6 in design/04 if it changes), with `rules.py`'s limits in mind (no issue numbers in design/, line length).

## Process
Use DeepSeek for the inventory sweep if it saves you context (tasks `ds-<n>-task.md`, logs `ds-<n>.log` in `~/.cache/ezsdr-fixes/2026-11-20-n120/`; a task file must not start with `---`; launch `cd ~/local_work/Ez-SDRv4-n111 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`, real PID via `pgrep -n -f 'deepseek-v4.1-flash#max'`, check `/proc/<PID>/cwd`; **wait with the Bash tool's `timeout` set to 600000**: `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`; tell DeepSeek: shell cuts at 120 s, read-only in phase A, no commit, no files outside the task) — but re-check its table yourself: a missed site is the typical failure. Adversarial self-review of the plan: `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.

## Report
Japanese, short: the plan's path; the inventory table's size and the sites it moves; the rule's text; #120's and #121's designs (one line each, plus rejected alternatives); open questions with your recommendation; INFERRED items.

## brief-v2-b1

# #120 + #121: plan v2, then sub-step B1 (the mechanical `Locked<T>` rewrite) — Sonnet drives Go DeepSeek

Same worktree, branch and rules as `~/.cache/ezsdr-fixes/2026-11-20-n120/brief.md` (read it; its Process applies; DeepSeek may now edit the tree in B1; never a mutant in the tree; rows via `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` from the worktree, `chmod 644` named files only; tests local).

1. **Plan v2**: write `test-plan-v2.md` (whole plan) applying every disposition in the last section of `decisions.md` (Sol's review `sol-plan.md`). F5 needs a design decision of yours (the accepted-but-unpublished handle when the freeze has passed): state it with the rejected alternative. No paper re-check follows; the orchestrator reads v2 and the sub-step reviews catch the rest.
2. **Sub-step B1** (code): `Locked<T>` (generalising `Serial<T>`) for every coordinator lock the rule covers, the mechanical `lock(&x)` → `x.lock()` rewrite, the lock records for the new locks, and the source-scan test — **no behaviour change** (the sweep's widening and #120/#121's fixes are B2/B3). `cargo fmt --all`; re-anchor every mutation row the rewrite touches (`rules.py --base cefd067c` must find each exactly once, the re-anchored row producing the same program); run the touched rows; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`.

Stop after B1 and report (Japanese, short): v2's path and its F1–F8 one line each (F5's design); B1's `git diff --stat`, the re-anchored rows and their outcomes, checks; deviations; INFERRED.

## brief-b2

# #120 + #121: sub-steps B2 and B3 — Sonnet drives Go DeepSeek

Same worktree, branch and rules as `brief.md` / `brief-v2-b1.md` in `~/.cache/ezsdr-fixes/2026-11-20-n120/`. Plan: `test-plan-v2.md` there, with `decisions.md` binding (all sections). B1 is committed (HEAD); Terra reviews it in parallel — if its review reports a defect, the orchestrator sends it to you.

- **B2**: the widened sweep and lock records (every watched Authority method recorded with the held locks; fixtures reach the paths they cite; the Q1 exception pinned on its own — T16, T19, M9 and the sweep changes T11–T14), plus #121 (copy out the links, `drops()` with `links` released, `link_drops` accepted only when step 6 completed, the `links` clear dropping outside the guard; §4.3 and F1's test). Stop, report, and wait for the orchestrator's commit before B3? **No — keep going**: write B2, run its checks and rows, then write a short B2 report section in `~/.cache/ezsdr-fixes/2026-11-20-n120/b2-report.md` and continue with B3 in the same tree (the orchestrator commits both at the end as two commits if you keep B2's and B3's files apart; otherwise one).
- **B3**: #120 (the `WakeList` design of decisions "Plan v2 + B1"; T17, T18, T20–T22, §4.2, F3's M7 killer, F7's test update) and the spec text (design/06 KC-24a rule and Ceiling, KC-44, KC-45, KC-46a; design/04 RS-6 steps 1/2 and 6 as the plan states; no issue numbers in design/; `rules.py` limits).

Every row dies by assertion; a hand-computed expectation that disagrees with a run stops you (report, never tune). Exit checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, new timing-dependent tests ×20 with all cores busy, `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base cefd067c`, the rows you add or touch. Adversarial self-review `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.

Report (Japanese, short): `git diff --stat` (and which files are B2 vs B3); per test and row the outcome; checks; deviations; INFERRED.

## brief-fix1

# #120 + #121 fix round 1 (after the Opus + Terra review of fbfae8b3) — Sonnet

Same worktree, branch and rules as `brief.md` / `brief-b2.md` in `~/.cache/ezsdr-fixes/2026-11-20-n120/`. HEAD = fbfae8b3 (committed). Fix exactly the adopted findings in the last section of `decisions.md` (reviews `opus-b23.md`, `terra-b23.md`). The main one changes production: the publisher that finds the wake list closed **cancels nothing** (drops its handle; the wake stays a no-op) — retarget T20 and its rows, and make RS-6 step 1 and KC-46a say the cutoff in the same words. Every new or retargeted row dies by assertion. A full mutation run of fbfae8b3 may be holding the sim queue; wait for it via `simq`. Exit checks as in brief-b2.md (`rules.py --base cefd067c --renames ~/.cache/ezsdr-fixes/2026-11-20-n120/renames.txt`), plus `cargo check -p ezsdr-kernel` without `testing` shows no warning.

Report (Japanese, short): `git diff --stat`; per finding the fix, test, row and outcome; checks; deviations.

## brief-fix2

# #120 + #121 fix round 2 (after Sol's code review) — Sonnet

Same worktree, branch and rules as `brief.md` / `brief-b2.md` in `~/.cache/ezsdr-fixes/2026-11-20-n120/`. HEAD = 465e5a9a (committed). Fix exactly the adopted findings in the last section of `decisions.md` (review `sol-code.md`). F1 and F2 change production (F2 in every build: a per-thread held-lock count in `Locked<T>`); write each failing test first (Sol's sequences), then the fix. Every new or retargeted row dies by assertion. Do not start a full mutation run. Exit checks as in brief-b2.md (`rules.py --base cefd067c --renames ~/.cache/ezsdr-fixes/2026-11-20-n120/renames.txt`), plus `cargo check -p ezsdr-kernel` without `testing` shows no warning.

Report (Japanese, short): `git diff --stat`; per finding the fix, test, row and outcome; checks; deviations.
