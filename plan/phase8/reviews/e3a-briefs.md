# E3a — briefs (2026-11-12)

## brief-plan

## Brief — note 49 stage E3, phase 1: the test plan (no code)

Work tree: **/home/komatsu/local_work/Ez-SDRv4-e3** (git worktree, branch `work/e3` from `main` `bdf3493`). Read only there; write only the plan file below. You are the E3 implementer (OpenCode Sonnet 5.5 #high); this phase is the test plan only.

Read first: AGENTS.md (§3, §7, §9 in full — E3 is **risky**: it moves the boundary between the fake/Mock and the hardware executor; the test plan comes before code and Sol reviews it); plan/phase8/49-time-and-booking-inventory.md (§5.4, especially §5.4.3 and §5.4.8 "Mock (M4)", the R/U/M tables in §3, §11.6–§11.8, §12's E3 row and the paragraph after §12's table: rows to retarget, pure transition tables, oracle and FakeDevice rules green throughout); the E2a and E2b records (plan/phase8/reviews/e2a-sketch.md, e2a-test-plan.md, e2a-dispositions.md, e2b-dispositions.md, e2b-test-plan.md); design/09-mock-radio.md MR-15, MR-25; design/18-uhd-radio.md UR-22, UR-23, UR-33; design/07-radio-model.md RM-15, RM-16. Code: `crates/ezsdr-radio/src/staging.rs` (`TxBook`), `crates/ezsdr-radio-uhd/src/provider/tx.rs` (the send split, first-send rule, padding, held tail, send sizes, cuts at a switch boundary), `crates/ezsdr-mock-radio/src/lib.rs` (issue and blocks MR-15, `tx_handed_end`), the O1 oracle `crates/ezsdr-acceptance/tests/oracle`, `crates/ezsdr-acceptance/tests/parity.rs`. Also plan/phase8/51-test-timing.md §3 (no test verdict may depend on when a thread runs).

### What E3 is (note 49 §12)

Move the pure segmentation decisions — the first-send rule, padding, the held tail, send sizes, the cut at a switch boundary — into `ezsdr_radio` as pure functions. The UHD state machine uses them for the real-time executor with the profile's device values. The Mock's stepped issue and blocks (MR-15) are not bound by the device's first-send minimum. The Mock's `tx_handed_end` (MR-25's second term, its model of what a real-time executor would have handed by a Stop) becomes a call of the same segmentation function without a send-size cap, with the Stop sample kept as the lower bound (§5.4.8). **Behaviour does not change** (MR-25 as today, also with W = 0); FakeDevice traces and the oracle stay green.

### The plan (write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3/test-plan.md)

1. **Inventory**: each decision to move, where it is today in UHD and in the Mock (path:line), whether the two copies agree today (if not, say how; that is a parity fact the move must keep or a bug to report, not to fix silently), and the existing tests that pin it.
2. **Characterization first**: for each decision, the observable behaviour to keep (FakeDevice `calls()` incl. send sizes/sob/eob/at, Manifest rows, Mock blocks and records), the test that pins it today or a planned characterization test green on `bdf3493` before any change, with a full ordered snapshot where a refactor could shift it.
3. **Sequences → tests** (each: the test, what it asserts, how it observes it, the wrong behaviour that fails it): pure tables for the moved functions (boundary values: 1–3, 4, 32, 33 samples, a held tail, a switch boundary inside a send, W = 0, a Stop sample before/after the first send would fit), UHD wiring/order tests, Mock tests including the MR-25 Stop cut with W = 0 and with the window, and parity tests where both run the same booking.
4. **Mutation rows**: for the new functions (Rust token anchors), the existing rows to retarget or re-anchor (grep the anchors of the code you will move; note 49 says many N47S5/N48S1/N48S3/N47S4 transmit rows), and the leftovers removed — a table old row → new anchor/killer.
5. **Hardware tests** whose expectations come from these rules: the AGENTS §9 audit table (test, expectation, still right / changes / cannot tell), and the FakeDevice rehearsals that must keep booking the same boundaries.
6. **Text**: `design/` rules that describe these decisions (MR-15, MR-25, UR-22, UR-23, UR-33, RM-15) and whether any wording must change (behaviour does not).
7. **Sub-steps**: propose a split (e.g. pure functions + characterization first, then UHD wiring, then the Mock) with the tree green after each.

Rules: no code edits, no builds in the tree (probes only in a copy outside the repository via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`, foreground); no commit. Final message: a summary of the plan, the sub-steps, and anything in note 49 or the E2a/E2b records the code contradicts.

## brief-plan2

## Brief — E3a, phase 1b: amend the test plan (no code)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3 (branch `work/e3`). You are the E3 implementer (OpenCode Sonnet 5.5 #high). Your first brief: /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3/brief-plan.md; your plan: test-plan.md there. Read sol-plan.md (Sol's review) and plan-decisions.md (the orchestrator's decisions: E3 split into E3a = this plan and E3b later; the K-points; and the section "After Sol's plan review") in the same directory. Amend test-plan.md in place so every decision and adopted finding is reflected, with a short "Changes after Sol's review" section at the top saying per finding what changed and where. Re-scope the plan's title and sub-steps to E3a.
Read-only everywhere else; no code; probes only in a copy via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`; no commit. Final message: per finding how it is closed, and anything you could not close with why.

## brief-impl1

## Brief — E3a-1: amend the plan for the re-check, then the characterization tests (test code only)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/e3` at `bdf3493` (the step's start). You are the E3 implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3/: brief-plan.md, test-plan.md (with "Changes after Sol's review"), sol-plan.md, plan-decisions.md, and the Opus re-check below (all its changes adopted, decided by the orchestrator).

### Opus re-check of the amended plan (no majors; changes needed)

- M1: write table Q relative to `t_k` and `we` (row 5's "finite 100 at 6 000" uses P-H's coordinates; both rigs stop at 20 000: `o1.rs:575-587`, Mock `o1.rs:656`); `before_stop` books finite bursts only (`o1.rs:579`) — add a repeat variant for rows 6–7; name the coverage loss instead of "nothing is lost" (the threaded production Session path for these shapes until note 51 stage 1b; two hand-typed copies of table Q — tie them with one shared literal if both test crates can reach it, else say so).
- M2 (MC5): the deadline literal is `v_after` (instant + one tick, `time.rs:30`); the Mock harness runs 1 000 root ticks a sample (Mock `o1.rs:7-8, 46`); `run_to(deadline − 1)` never steps (`mock_channel.rs:313-320`) — call `step` at deadline − 1; state that the transmit wake is the minimum at the chosen positions (a loopback receive stream also wakes).
- m3 (MC4): take a row only when `tx_blocks` grows; a Stop's non-EOB cut **is** recorded (`BurstTracker::stop`, MR-15) — fix the wording; the row's record `end` distinguishes the cut.
- m4: P39 is equivalent while `handed_end` keeps the `start < we` guard (`lib.rs:290` with `:1404`) — drop the redundant guard or retire P39; P41's killer is the "finite 100 at t_k-relative" row, rename it; add a Mock call-site row for passing rules other than `PACKETS` (killer `mr_25_a_repeat_first_send_is_judged_at_32_samples`); add a UHD row for passing `first` in place of `device_sob` (`tx.rs:654`) into `plan_send` (killer C3 or `ur_23_m4_…`).
- m5: R4 asserts the expected two-sample held-tail close occurred (not vacuous on a direct EOB close); R1–R4 run on the real-clock `FakeConfig::default()` in a threaded Session — R3's "booked after A", "no TIME_ERROR" and "equal ack" are window judgments: drop them or back them with a receipt record (note 51 §3 P3, P7, contract 5); mark R1–R4 under P4 and never name them as a mutation row's killer.

### Then: E3a-1 (plan §7)

Only test code: the characterization tests C1–C6 (UHD, full ordered snapshots), MC1–MC5 (Mock), U1 and table Q, the rehearsals R1–R4, and C5 for `open_ended`'s boundary — **each green on today's code `bdf3493`**. Run each twice on sim03 through rdev; a field that differs between runs is named and excluded with its reason; timing-dependent tests 20 times. No production change and no mutation rows yet (they come with the code). A planned expectation today's code contradicts is reported, not fixed.

### Rules

Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). Builds/tests via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>` (wait in the foreground); temp files on /tmp; kill by PID only; never a mutant in the tree. `cargo fmt --all`; `rdev -- cargo test --workspace` green; `rdev -- cargo clippy --workspace --all-targets` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base bdf3493` = 0. No commit.

### Report

`git diff --stat`; per test: green ×2 (×20 where timing-dependent), what it pins, exclusions; the plan amendments; contradictions with the plan.

## brief-impl2

## Brief — E3a-2 (see test-plan.md §7, row E3a-2)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/e3` (step start `bdf3493`; earlier sub-steps are local commits — `git log bdf3493..HEAD`). You are the E3 implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3/: brief-plan.md, test-plan.md (all its "Changes" sections), plan-decisions.md, sol-plan.md, brief-impl1.md and the previous sub-steps' briefs. Do **only row E3a-2** of plan §7, exactly as the row states (its content, its rows, its review checklist), and the plan's trace comparison where the row names it.

Rules: no behaviour change; every E3a-1 characterization test (C1–C6, U1/U1b, MC1–MC5, R1–R4) stays green **with its expectations unedited**; P-G stays unchanged until E3a-5. Mutation rows: re-anchor what your edit breaks (same program change, same Provider; never move a UHD-only or Mock-only rule into shared code; no two rows identical; every `new` compiles); run the rows you add or touch through `remote_mutate.py <list> sim03:4 <ids>` (the other lane may run the full lists on both hosts — if so, wait for it rather than overlap). Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). Builds via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`, foreground; copies under /tmp with `CARGO_INCREMENTAL=0`, target outside the tree, deleted after; never a mutant in the tree; kill by PID only. `cargo fmt --all`; `rdev -- cargo test --workspace` green; `rdev -- cargo clippy --workspace --all-targets` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base bdf3493` = 0. Add a "Changes in E3a-2" section to test-plan.md for anything that differs from the plan. No commit.

Report: `git diff --stat`; what the row required and what you did; rows and outcomes; the trace comparison result if the row names it; deviations with reasons.

## brief-impl3

## Brief — E3a-3 (see test-plan.md §7, row E3a-3)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/e3` (step start `bdf3493`; earlier sub-steps are local commits — `git log bdf3493..HEAD`). You are the E3 implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3/: brief-plan.md, test-plan.md (all its "Changes" sections), plan-decisions.md, sol-plan.md, brief-impl1.md and the previous sub-steps' briefs. Do **only row E3a-3** of plan §7, exactly as the row states (its content, its rows, its review checklist), and the plan's trace comparison where the row names it.

Rules: no behaviour change; every E3a-1 characterization test (C1–C6, U1/U1b, MC1–MC5, R1–R4) stays green **with its expectations unedited**; P-G stays unchanged until E3a-5. Mutation rows: re-anchor what your edit breaks (same program change, same Provider; never move a UHD-only or Mock-only rule into shared code; no two rows identical; every `new` compiles); run the rows you add or touch through `remote_mutate.py <list> sim03:4 <ids>` (the other lane may run the full lists on both hosts — if so, wait for it rather than overlap). Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). Builds via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`, foreground; copies under /tmp with `CARGO_INCREMENTAL=0`, target outside the tree, deleted after; never a mutant in the tree; kill by PID only. `cargo fmt --all`; `rdev -- cargo test --workspace` green; `rdev -- cargo clippy --workspace --all-targets` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base bdf3493` = 0. Add a "Changes in E3a-3" section to test-plan.md for anything that differs from the plan. No commit.

Report: `git diff --stat`; what the row required and what you did; rows and outcomes; the trace comparison result if the row names it; deviations with reasons.

## brief-impl4

## Brief — E3a-4 (see test-plan.md §7, row E3a-4)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/e3` (step start `bdf3493`; earlier sub-steps are local commits — `git log bdf3493..HEAD`). You are the E3 implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3/: brief-plan.md, test-plan.md (all its "Changes" sections), plan-decisions.md, sol-plan.md, brief-impl1.md and the previous sub-steps' briefs. Do **only row E3a-4** of plan §7, exactly as the row states (its content, its rows, its review checklist), and the plan's trace comparison where the row names it.

Rules: no behaviour change; every E3a-1 characterization test (C1–C6, U1/U1b, MC1–MC5, R1–R4) stays green **with its expectations unedited**; P-G stays unchanged until E3a-5. Mutation rows: re-anchor what your edit breaks (same program change, same Provider; never move a UHD-only or Mock-only rule into shared code; no two rows identical; every `new` compiles); run the rows you add or touch through `remote_mutate.py <list> sim03:4 <ids>` (the other lane may run the full lists on both hosts — if so, wait for it rather than overlap). Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). Builds via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`, foreground; copies under /tmp with `CARGO_INCREMENTAL=0`, target outside the tree, deleted after; never a mutant in the tree; kill by PID only. `cargo fmt --all`; `rdev -- cargo test --workspace` green; `rdev -- cargo clippy --workspace --all-targets` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base bdf3493` = 0. Add a "Changes in E3a-4" section to test-plan.md for anything that differs from the plan. No commit.

Report: `git diff --stat`; what the row required and what you did; rows and outcomes; the trace comparison result if the row names it; deviations with reasons.

## brief-impl5

## Brief — E3a-5 (test-plan.md §7, row E3a-5), except the gate

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/e3` (step start `bdf3493`; sub-steps E3a-1…4 are local commits). You are the E3 implementer (OpenCode Sonnet 5.5 #high). The rules of brief-impl2.md (same directory) bind. Do row E3a-5 except the gate and the closing records (the orchestrator runs the full lists, Sol's code review and the gate):
- delete P-G (the transitional differential copies) and the rows that only it killed, if any (say which);
- fix the stale cites the plan lists (note 49 §3/§5.4.10 line cites are plan/ text — list them for the orchestrator, do not edit plan/ files other than test-plan.md);
- the final trace comparison over both Providers (UHD lib and the Mock/acceptance set), base `bdf3493` vs HEAD, four runs each — expected zero differing files on the stable set;
- note 49 §5.4.8's wording about MR-25's second term (K5: an open booking counts with a first send of one sample; the term ignores the next held start; a joined piece is a fresh first send) — write the proposed sentence in test-plan.md for the orchestrator;
- `cargo fmt --all`; `rdev -- cargo test --workspace` green; clippy clean (also `--features uhd` for ezsdr-radio-uhd); `rules.py --base bdf3493` = 0. No commit.
Report: `git diff --stat`, the trace result, the deleted rows, the text proposals.

## Orchestrator decisions on the test plan

## E3 — orchestrator decisions on the test plan's open points (2026-11-12, decided by the orchestrator)

- K1 scope: the E2a sketch (§2 item 2, §7: "E3 must move TxChain and the segmentation into the shared function together"; W5) and note 49 §12 disagree. Both are kept, staged to control risk: **E3a** = this plan (pure segmentation functions in `ezsdr_radio`, UHD and Mock wired to them, behaviour unchanged); **E3b** = moving `TxChain` (device-burst state) into the book and replacing the record's front prediction (`staging.rs` `front()`) and the stat `tx_close_past_record_front` with the shared segmentation, with its own plan. Rationale: one front arithmetic is the long-term shape (D′), but E3a alone is already the largest re-anchoring of transmit rows; doing both at once is the riskiest stage the E2a sketch warned about.
- K2/K7: the packet values live in `ezsdr_radio::x3xx` as one `PacketRules` (one field for the 32 used by both the short-first-send and the short-gap decisions); FakeDevice keeps its own literals (K3: the fake and the Module become independent for the 4).
- K4: the Mock's Stop model uses the window-only mode, not an infinite cap.
- K5: the parity facts F3a/F3b are kept and pinned; note 49 §5.4.8's wording is amended at landing.
- K8: duplicate rows P714/P1160 — remove one; P1111/P1112 — keep both only if their killers differ in what they observe, else remove one.
- K9: add the two FakeDevice rehearsals in E3a-1. K10: C5 and rows for `open_ended`'s boundary.
- `handover_horizon` (`tx.rs:487`): reuse it if it is the same decision; do not create a second copy.

### After Sol's plan review (sol-plan.md: FAIL, 2 majors, 3 minors) — all adopted (decided by the orchestrator)
- M1 Q1/Q2: no margins or repetitions as the timing guarantee (note 51 §3; `UhdRig`'s driver is not a manual-clock participant until note 51 stage 1b). Use the frozen/stepped per-Provider rigs (O1's frozen UHD rig and the Mock loopback) with the Stop received at a stated device instant, or judge from independently observed receipt/application times. If a paired comparison cannot be made timing-independent before stage 1b, drop it from E3a and say what covers the property instead.
- M2 MC5: observe the scheduled wake deadline (or drive through the Authority's next wakeup) and assert both the deadline and the resulting block.
- m3 MC4: ordered `(first_sample_time, len, flags)` snapshots for finite end, wrap, held start, clock cut, Stop cut (EOB vs non-EOB cut).
- m4: replace the equivalent mutants with observable ones; an explicit old-row → replacement/killer table; keep Mock-only thresholds and reductions at their own call sites.
- m5: audit rows for `hw_p70_one_sample_burst_then_timed_burst` and `hw_p72_…`, with named rehearsals or added coverage.
- Keep MR-25's window-only mode free of real-time held-start/wrap restrictions; the verbatim differential copies stay frozen through both wiring sub-steps.
