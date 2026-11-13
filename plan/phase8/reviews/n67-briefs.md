# #67 — briefs (2026-11-13)

## brief-plan

# Brief — #67: uhd-rx outlives its 1 s join when Provider::stop meets a timed start queued for a distant origin — test plan (plan only)

Worktree /home/komatsu/local_work/Ez-SDRv4-e3, a new branch `work/n67` from main (the orchestrator creates it). You are the implementer (OpenCode Sonnet 5.5 #high). This run writes the **test plan only**, to /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n67/test-plan.md; no edits in the tree. Probes only in a copy outside it (`rsync -a --no-times --exclude .cargo --exclude target <wt>/ /tmp/n67-probe/`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`), deleted at the end.

## The issue
`gh issue view 67`. A segment begun well ahead of its origin has its timed rx_start queued in the device; `Provider::stop`'s End cuts it at its origin; under VH-6 (plan/maintenance/22-timing-simplification.md §VH-6, written when the hardware answer to #56 was unknown) uhd-rx waits for the origin, stops untimed and discards what arrives (`crates/ezsdr-radio-uhd/src/provider/rx.rs` `follow`, `begin`). With an origin > ~1 s away, `UhdRadio::stop` records `did not join within 1 s; left detached`, cleanup skips `close_streams` (UR-16): streamers leak, the thread lives until the origin. Mechanism `stream-lifecycle-scattered` (plan/maintenance/failure-mechanisms.md).

## Hardware evidence (bench 2026-11-13, VERIFIED, bench/2026-11-13-n67-records, `hw_67_stop_before_origin`, both boards, 1/10/100 MS/s × 10)
- An untimed rx_stop issued before the origin cancels the queued timed start: no block of it ever arrives (30/30 per board); a fresh timed start afterwards delivers its first block at its own origin.
- A close and reopen without a stop does NOT cancel it: the old start's blocks arrive on the reopened streamer at the old origin (30/30 FAIL per board).
- (UHD 4.10 source, Sol VERIFIED earlier: STOP bypasses and clears the command FIFO.)

## Direction (orchestrator; risky step: a lifecycle and the fake/hardware boundary)
A cut at or before a queued start's origin is ended at once by an untimed rx_stop (then drain, then end) instead of waiting for the origin — at least for `Provider::stop`; decide in the plan, with the reasons, whether an ordinary Kernel `Stop` that cuts a not-yet-started segment gets the same (it simplifies VH-6's wait; check RM/UR rules and the tests that pin the wait). The shutdown then joins well within 1 s and closes the streamers. Never close a streamer that has a queued start without stopping it first (the reopen evidence).

## The plan must cover (each: test, what it asserts, how it observes it, the wrong behaviour that fails it)
- Provider::stop with a queued start 5 s ahead: join within the bound, streamers closed (UR-16), no `left detached` row, no samples delivered from the cancelled start, the device saw rx_stop before rx_close.
- The same with the origin already passed (today's path, unchanged), and with the origin just ahead (< the join bound).
- An ordinary Stop cutting a queued segment (per your decision), then `start_rx` again: the fresh start's first block at its origin, nothing from the old one.
- A device loss during that window; a Stop and Provider::stop racing.
- The FakeDevice must model the hardware: an untimed rx_stop before the origin cancels the queued start; a close/reopen without a stop does not (the old start's data arrives at its origin on the new streamer). Check what the fake does today (src/device.rs) and plan the change; a default-suite rehearsal books the same boundary as `hw_67_stop_before_origin` (AGENTS §9 hardware tests rule).
- MockRadio: does it have the same wait? (crates/ezsdr-mock-radio) — the rule both Providers follow must stay one.
- Text to change: VH-6's sentence where it lives in `design/` now (search), UR-16/UR-17/RM-21 if they say "waits for the origin", KNOWN_LIMITATIONS if listed; failure-mechanisms row.
- Mutation rows (Rust token anchors) and the hardware audit table for every hw_ test whose expectation changes.

You may offload mechanical reading or edits to Go DeepSeek yourself (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`), at your judgment; DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries), so check what it returns. Foreground, one at a time; list what you offloaded.

Report: the path and a short summary; open questions for the orchestrator with your recommendation; anything INFERRED.

## brief-impl

# Brief — #67 implementation

Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n67` at main `79f55e1`. You are the implementer (OpenCode Sonnet 5.5 #high). Self-review is waived; Sol reviews the code at the end. Risky step (a lifecycle and the fake/hardware boundary).

Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n67/: test-plan.md (yours), sol-plan.md, plan-decisions.md (every section binds; the last one replaces D3 with **stop on open** and adopts all minors). First add a short "Changes after Sol's plan review" section to test-plan.md (stop on open with its tests and the hw_67 CloseThenStop cell; per-channel orphans; C1 fixture; rehearsal timing; the audit row), then implement in three sub-steps:
- **A**: the `done` conjunct in rx.rs, the uhd-rx unit tests (U1–U9), the differential's scripted device and compare, run.rs's `rx_join_issue_67` removal (P1), the Provider-level P2–P5.
- **B**: FakeDevice per-channel orphans (D1–D5), stop on open (Module) with its tests, the rehearsal and `hw_67_stop_before_origin` with `expected_cancel` and the CloseThenStop cell.
- **C**: text (design/18 UR-16/UR-17/UR-33 and the test table and Changes row, VH-6's superseded note, failure-mechanisms, tests/README) and the remaining rows (G60 retarget).

Rules:
- Local cargo builds and tests are fine (no bench runs today). Mutation rows only through `~/.cache/ezsdr-fixes/tools/simq env REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <list> sim02:8,sim03:8 <ids...>`, in the foreground.
- Per sub-step: tests green for the crates you touched, the sub-step's rows killed, `cargo fmt --all`, `python3 plan/maintenance/tools/rules.py --base 79f55e1` 0 problems; write `sub-step-<X>-report.md` in the directory above and **exit** (the orchestrator commits each sub-step and relaunches you). Do not commit.
- Every planned sequence gets its test, every test its killing row (Rust token anchors, smallest unique text); the hardware audit table for every hw_ test whose expectation changes. New tests whose verdict can depend on timing: 20 serial runs.
- Do not touch MockRadio or `ezsdr_radio::timeline` (D2). No mechanisms beyond the plan and the decisions.
- You may offload mechanical work to Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`), at your judgment. DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries): read its diff and run the tests before you accept it. Foreground, one at a time, not touching the tree meanwhile; list what you offloaded.
- Never edit /home/komatsu/local_work/Ez-SDRv4 or Ez-SDRv4-a1. Test temp files on /tmp. Kill processes only by PID.

Report per sub-step: files changed, tests run (command + result), rows run (ids + outcome), plan deviations with reasons, anything INFERRED.

## brief-impl-B

# Brief — #67 implementation

Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n67` at main `79f55e1`. You are the implementer (OpenCode Sonnet 5.5 #high). Self-review is waived; Sol reviews the code at the end. Risky step (a lifecycle and the fake/hardware boundary).

Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n67/: test-plan.md (yours), sol-plan.md, plan-decisions.md (every section binds; the last one replaces D3 with **stop on open** and adopts all minors). First add a short "Changes after Sol's plan review" section to test-plan.md (stop on open with its tests and the hw_67 CloseThenStop cell; per-channel orphans; C1 fixture; rehearsal timing; the audit row), then implement in three sub-steps (A is done and committed on work/n67; do B now, then exit):
- **A**: the `done` conjunct in rx.rs, the uhd-rx unit tests (U1–U9), the differential's scripted device and compare, run.rs's `rx_join_issue_67` removal (P1), the Provider-level P2–P5.
- **B**: FakeDevice per-channel orphans (D1–D5), stop on open (Module) with its tests, the rehearsal and `hw_67_stop_before_origin` with `expected_cancel` and the CloseThenStop cell.
- **C**: text (design/18 UR-16/UR-17/UR-33 and the test table and Changes row, VH-6's superseded note, failure-mechanisms, tests/README) and the remaining rows (G60 retarget).

Rules:
- Local cargo builds and tests are fine (no bench runs today). Mutation rows only through `~/.cache/ezsdr-fixes/tools/simq env REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <list> sim02:8,sim03:8 <ids...>`, in the foreground.
- Per sub-step: tests green for the crates you touched, the sub-step's rows killed, `cargo fmt --all`, `python3 plan/maintenance/tools/rules.py --base 79f55e1` 0 problems; write `sub-step-<X>-report.md` in the directory above and **exit** (the orchestrator commits each sub-step and relaunches you). Do not commit.
- Every planned sequence gets its test, every test its killing row (Rust token anchors, smallest unique text); the hardware audit table for every hw_ test whose expectation changes. New tests whose verdict can depend on timing: 20 serial runs.
- Do not touch MockRadio or `ezsdr_radio::timeline` (D2). No mechanisms beyond the plan and the decisions.
- Never edit /home/komatsu/local_work/Ez-SDRv4 or Ez-SDRv4-a1. Test temp files on /tmp. Kill processes only by PID.

Report per sub-step: files changed, tests run (command + result), rows run (ids + outcome), plan deviations with reasons, anything INFERRED.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits across files, renames, new tests that follow an existing pattern, mutation rows, text updates — and keep your own effort for design, review and the hard parts. What to hand over is your call. DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries): read its diff and run the tests before you accept it. Run it in the foreground, one at a time, and do not touch the tree meanwhile; list what you offloaded in your report.

Also read sub-step-A-report.md and the last section of plan-decisions.md. Write sub-step-B-report.md and exit.

## brief-impl-C

# Brief — #67 implementation

Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n67` at main `79f55e1`. You are the implementer (OpenCode Sonnet 5.5 #high). Self-review is waived; Sol reviews the code at the end. Risky step (a lifecycle and the fake/hardware boundary).

Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n67/: test-plan.md (yours), sol-plan.md, plan-decisions.md (every section binds; the last one replaces D3 with **stop on open** and adopts all minors). First add a short "Changes after Sol's plan review" section to test-plan.md (stop on open with its tests and the hw_67 CloseThenStop cell; per-channel orphans; C1 fixture; rehearsal timing; the audit row), then implement in three sub-steps (A and B are done and committed on work/n67; do C now, then exit):
- **A**: the `done` conjunct in rx.rs, the uhd-rx unit tests (U1–U9), the differential's scripted device and compare, run.rs's `rx_join_issue_67` removal (P1), the Provider-level P2–P5.
- **B**: FakeDevice per-channel orphans (D1–D5), stop on open (Module) with its tests, the rehearsal and `hw_67_stop_before_origin` with `expected_cancel` and the CloseThenStop cell.
- **C**: text (design/18 UR-16/UR-17/UR-33 and the test table and Changes row, VH-6's superseded note, failure-mechanisms, tests/README) and the remaining rows (G60 retarget).

Rules:
- Local cargo builds and tests are fine (no bench runs today). Mutation rows only through `~/.cache/ezsdr-fixes/tools/simq env REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <list> sim02:8,sim03:8 <ids...>`, in the foreground.
- Per sub-step: tests green for the crates you touched, the sub-step's rows killed, `cargo fmt --all`, `python3 plan/maintenance/tools/rules.py --base 79f55e1` 0 problems; write `sub-step-<X>-report.md` in the directory above and **exit** (the orchestrator commits each sub-step and relaunches you). Do not commit.
- Every planned sequence gets its test, every test its killing row (Rust token anchors, smallest unique text); the hardware audit table for every hw_ test whose expectation changes. New tests whose verdict can depend on timing: 20 serial runs.
- Do not touch MockRadio or `ezsdr_radio::timeline` (D2). No mechanisms beyond the plan and the decisions.
- Never edit /home/komatsu/local_work/Ez-SDRv4 or Ez-SDRv4-a1. Test temp files on /tmp. Kill processes only by PID.

Report per sub-step: files changed, tests run (command + result), rows run (ids + outcome), plan deviations with reasons, anything INFERRED.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits across files, renames, new tests that follow an existing pattern, mutation rows, text updates — and keep your own effort for design, review and the hard parts. What to hand over is your call. DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries): read its diff and run the tests before you accept it. Run it in the foreground, one at a time, and do not touch the tree meanwhile; list what you offloaded in your report.

Also read sub-step-A-report.md, sub-step-B-report.md and the last two sections of plan-decisions.md. In C also: KNOWN_LIMITATIONS needs no entry (none for #67); add the Changes row to design/18 dated 2026-11-13 linking issue #67 and plan/phase8/reviews/n67-* (the orchestrator files them); run `cargo test -p ezsdr-radio-uhd -p ezsdr-acceptance` once at the end. Write sub-step-C-report.md and exit.

## brief-fix1

# Brief — #67 fix round 1 (Sol code review)

Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n67` (A, B, C committed; head 0270754). You are the implementer. Read /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n67/sol-code.md and the last section of plan-decisions.md there: fix M1, m2, m3, m4 as written. Each fix gets its test first (red before the fix where practical) and its killing row (N67F-27…, Rust token anchors). Update design/18 where a sentence becomes false (m4; UR-16/UR-29 for M1 if needed) and the test tables.

Rules: local cargo in the worktree with NO CARGO_TARGET_DIR override (its .cargo/config.toml decides); rows only via `~/.cache/ezsdr-fixes/tools/simq env REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:8,sim03:8 <ids>`, foreground; run the touched rows too. Then `cargo test -p ezsdr-radio-uhd -p ezsdr-acceptance`, `cargo fmt --all`, `python3 plan/maintenance/tools/rules.py --base 79f55e1 --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n67/renames.txt` 0 problems, `cargo clippy -p ezsdr-radio-uhd -p ezsdr-acceptance --all-targets -- -D warnings`. New tests whose verdict can depend on timing: 20 serial runs. Do not commit. Never edit /home/komatsu/local_work/Ez-SDRv4 or Ez-SDRv4-a1. Test temp files on /tmp; kill processes only by PID.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`) costs about a tenth of Sonnet 5.5 and is fast, so use it for the bulk of the hands-on work you can describe precisely — repetitive edits across files, renames, new tests that follow an existing pattern, mutation rows, text updates — and keep your own effort for design, review and the hard parts. What to hand over is your call. DeepSeek makes mistakes (adds unasked mechanisms, misreads concurrency and fake/hardware boundaries): read its diff and run the tests before you accept it. Run it in the foreground, one at a time, and do not touch the tree meanwhile; list what you offloaded in your report.

Write /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n67/fix1-report.md (files, tests and results, rows and outcomes, deviations, INFERRED) and end with a short summary.

## brief-fix2

# Brief — #67 fix round 2 (minor-only)

Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/n67` (fix round 1 committed as 6e0a460). You are the implementer. Do exactly the last section of /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n67/plan-decisions.md: restrict `refuse_open` to the receive open at `arm` (provider/mod.rs:574 back to `rejected(open_refusal(e))`), and the listed text/comment/anchor nits. Re-run the rows whose anchors you touch and any row on `refuse_open` via `~/.cache/ezsdr-fixes/tools/simq env REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:8,sim03:8 <ids>` (foreground). Then `cargo test -p ezsdr-radio-uhd`, `cargo fmt --all`, `python3 plan/maintenance/tools/rules.py --base 79f55e1 --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n67/renames.txt`, clippy -D warnings for ezsdr-radio-uhd. Local commands with no CARGO_TARGET_DIR override. Do not commit; never edit the other two trees; kill processes only by PID; temp files on /tmp. Write fix2-report.md in that directory and end with a short summary.
