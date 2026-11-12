# ===== brief-plan.md

# Brief — note 49 stage E2b, phase 1: the test plan (no code)

Work tree: **/home/komatsu/local_work/Ez-SDRv4-e2b** (git worktree, branch `work/e2b` from `main` `9c84502`). Read only there; write only the plan file below. You are the implementer for E2b (OpenCode Opus 5.5); this phase is the test plan only.

Read first: AGENTS.md (§3, §7, §9 in full — E2b is **risky**: it changes a lifecycle; the test plan comes before code and Sol reviews it); plan/phase8/49-time-and-booking-inventory.md (§5.4, especially §5.4.6 "configure is the precondition of payload issue", and §12's E2b row: "`Configure` and `Reopen` become operations; the precondition of payload issue; re-checked inside the lock after the result — behaviour unchanged (the same order as today's configure turn)"); the E2a records plan/phase8/reviews/e2a-sketch.md (§2 items 2 and 5, the W6/W7 rows, "(d1)", and the lines naming E2b), e2a-test-plan.md and e2a-dispositions.md; the code: `crates/ezsdr-radio/src/staging.rs` (`TxBook`), `crates/ezsdr-radio-uhd/src/provider/tx.rs` (the configure turn ~:538, :614-641; the reopen turn ~:923-995; `record_configured` ~:627, :629, :952; `drop_piece` ~:636; `cancel_switch` ~:990), `control.rs`, `core.rs`; and how MockRadio does the same steps (`crates/ezsdr-mock-radio/src/lib.rs`). Also plan/phase8/51-test-timing.md §3 (the test-timing rules: no test's verdict may depend on when a thread runs; use the manual clock, frozen rigs and observed records).

## What E2b is

Bring the configuring booking into the book (today it leaves `held` during the configure turn) and make `Configure` and `Reopen` operations of the book's protocol (issued with a token, their result applied once by `complete`), as E2a did for `Send`: the record writes of the configure and reopen turns (`configure_refused` = today's `drop_piece`, `clock_ended` = today's `cancel_switch` on a failed reopen) become `complete(Configure)` / `complete(Reopen)` transitions; payload issue requires a completed Configure; the state is re-checked inside the lock after each result. **Behaviour does not change**: every device call order, record row, event and refusal stays as today. Decide, with evidence, whether Close also becomes an op now ((d1) left it for E2b "or when C2/C6 need a Close result") — recommend the smaller choice.

## The plan (write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e2b/test-plan.md)

1. **Characterization first**: list today's observable behaviour for each sequence below (device call order in `FakeDevice::calls()`, Manifest rows, events, refusals), from the code with path:line, and the existing test that pins it, or "none" (then plan a characterization test that passes on today's code before any change).
2. **Sequences** (each: the test, what it asserts, how it observes it, the wrong behaviour that would fail it). At least:
   - a booking that needs a configure: configure succeeds → payload issued after the result, never before; the configure row;
   - configure refused → the refused booking dropped as today, later bookings as today (KL-38 unchanged), no payload issued;
   - a Stop received while the configure is in flight (before and after its result); `Provider::stop` during it; a loss during it;
   - a switch received during a configure turn;
   - reopen succeeds; reopen fails → the clock ends as today (`cancel_switch`), later bookings refused as today;
   - a Stop / loss / `Provider::stop` during a reopen;
   - two bookings with different settings back to back (configure between them), and one with the same settings (no configure);
   - the result arriving after the book changed (re-check inside the lock).
   Use pure book-level tests for the transitions and keep device-path tests for wiring and ordering (AGENTS §9 Maintainability 3). Mock parity where the Mock has the same step.
3. **Mutation rows** for the new transitions (Rust token anchors), the rows that move or retarget (list the existing rows anchored on the code you will change: `grep` the anchors), and the removed leftovers.
4. **Hardware tests** whose expectations depend on these turns: the AGENTS §9 audit table (test, expectation, still right / changes / cannot tell).
5. Text: `design/` rules that describe the configure/reopen turns (search UR-*, RM-*, MR-*), and whether any must change (behaviour does not, so only mechanism wording may).

Rules: no code edits, no builds in the tree (probes only in a copy outside the repository via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`, long explicit shell timeouts, foreground); no commit. Final message: a summary of the plan, the Close-op recommendation, and anything in note 49 or the E2a records the code contradicts.

# ===== brief-plan2.md

# Brief — E2b, phase 1b: amend the test plan after Sol's review (no code)

Work tree /home/komatsu/local_work/Ez-SDRv4-e2b (branch `work/e2b`). You are the E2b implementer (OpenCode Opus 5.5). Your first brief: /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e2b/brief-plan.md; your plan: test-plan.md in that directory. Read Sol's review sol-plan.md and the orchestrator's adjudication plan-adjudication.md (both there). Amend test-plan.md in place so every adopted finding is closed: say per finding what changed (a short "Changes after Sol's review" section at the top). Keep the rest of the plan unless a finding requires a change. The two owner points stay open; keep today's behaviour for both in the plan.
Read-only everywhere else; no code, builds only in a copy via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>` if a probe is needed; no commit. Final message: per finding, how it is closed (plan section), and anything you could not close with why.

# ===== brief-impl1.md

# Brief — E2b-1: the characterization tests and the test seam (test code only)

Work tree /home/komatsu/local_work/Ez-SDRv4-e2b, branch `work/e2b` at `9c84502` (the step's start commit). You are the E2b implementer (OpenCode Opus 5.5). E2b is risky (a lifecycle); its test plan is reviewed.

Read first: /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e2b/{brief-plan.md, test-plan.md (with "Changes after Sol's review"), sol-plan.md, plan-adjudication.md (orchestrator and owner decisions: tokens on Send, Configure and Reopen; loss mid-configure kept as today and filed as #98; failed reopen kept as today), recheck-plan.md (the Opus re-check: M1 closed with changes, M2/M3 closed, new N1–N6)}.

## First: amend test-plan.md for the re-check (decided by the orchestrator)

- N1: `take` and `end_open` assert that no operation is in flight (production never ends an open booking with a Send in flight: tx.rs:879, 992, 1036). Amend `txbook_take_untake_and_end_open_keep_one_open_booking_and_change_no_record` (crates/ezsdr-radio/tests/staging.rs:1748-1762) so it completes S1 before `end_open`, and list it as a changed test with this reason; add a pure test that `end_open` with an op in flight panics, with a row.
- N2: one test function per `should_panic` case (or `catch_unwind` per case with the exact message), so each of E2B-03 / E2B-07 has its own killer.
- N3: add C9(c): a failed reopen (`TxOpenFails`) with the hook's loss before the result section → no ClockEnded rejection; plus a row hoisting that loss read.
- N4: C7, C9(a) and R2b run on a `released` participant thread → repeat each new timing-dependent test 20 times (AGENTS §9 step 8) before reporting.
- N5: C8 asserts the hook ran and the slot is empty before Z′ is booked; the test thread holds no `Control` guard during `turns`.
- N6: add a row moving `issue_reopen` before the stop/loss/claim early returns (tx.rs:930-940), or state which existing test kills it.

## Then: this sub-step (plan §3.3 "the first edit")

Only test code: the `on_next_lock` seam (test build only, in `lock`'s `#[cfg(test)]` block, with its self-check), `crates/ezsdr-radio-uhd/src/provider/tx/tests/e2b.rs` with C0–C9 (each with its full ordered snapshot: device calls, `applied`, `bursts`, `rejected`, `switches`, `timing`, `device_async`, events with payloads; state any exclusion), R2a and R2b. **Every test is green on today's production code** — this pins today's behaviour before E2b changes it. Run each twice on sim03 through rdev (and the timing-dependent ones 20 times); a field that differs between runs is named and excluded with its reason. No production change, no mutation rows yet (they come with E2b-2's code).

## Rules

- Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3): manual clock, frozen rigs, `released`, observed records; no wall-clock sleeps or bounds as oracles.
- Builds and tests: `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>` (another lane may run mutation on both hosts: if rdev is slow, wait, do not run on usrp-lnx03 under load). Mutation rows only through `remote_mutate.py` (not needed in this sub-step). Never apply a mutant to the tree. Temp files on /tmp; kill by PID only.
- `cargo fmt --all`; `rdev -- cargo test --workspace` green; `rdev -- cargo clippy --workspace --all-targets` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 9c84502` = 0. Wait in the foreground for every run. No commit.

## Report

`git diff --stat`; per C/R test: green ×2 (×20 where timing-dependent), what its snapshot covers, exclusions; the plan amendments for N1–N6; anything in the plan that today's code contradicts.

# ===== brief-impl2.md

# Brief — E2b-2: `Configure` and `Reopen` become operations of the book, with tokens (production change, no behaviour change)

Work tree /home/komatsu/local_work/Ez-SDRv4-e2b, branch `work/e2b`. Step start `9c84502`; E2b-1 is committed locally as `2e53a76` (the characterization tests C0–C9, R2a/R2b and the `on_next_lock` seam — they pin today's behaviour and must stay green **without editing their expected snapshots**). Implementer: OpenCode Sonnet 5.5 #high.

Read first, in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e2b/: brief-plan.md (the step), test-plan.md (the reviewed plan — all its "Changes …" sections, including "Changes after the Opus re-check" and "Changes in E2b-1"), plan-adjudication.md (decisions: tokens on Send, Configure and Reopen; #98 and the failed-reopen case kept as today), recheck-plan.md, sol-plan.md; then `git show 2e53a76`; note 49 §5.4 (plan/phase8/49-time-and-booking-inventory.md); AGENTS.md §3, §7, §9.

## Scope (plan §0.2, §3.2, §3.3, §4)

- The book holds the configuring booking (it no longer leaves `held` during the configure turn) and one in-flight operation: Send, Configure or Reopen, each issued with a token (`OpId`, unique) and completed by naming it. Stale, duplicate, wrong-token or wrong-kind completion panics before changing anything. `take` and `end_open` assert nothing is in flight (N1).
- `complete(Configure)` carries today's record writes (`configure_refused` = today's `drop_piece`); `complete(Reopen)` carries today's (`clock_ended` on a failed reopen). Payload issue requires a completed Configure. The state is re-checked inside the lock after each result.
- **Keep today's critical sections and their order** (Sol M2): the mark section, then the book-result section, exactly where today's sections are; the loss read stays inside the result section (M3, C9). The Mock respells its Send pair to the token API.
- **No behaviour change**: every E2b-1 test stays green with its stored snapshot unchanged; every existing test stays green with no assertion edited, except the one the plan lists as changed (N1).
- Pure tests P1–P8 in `crates/ezsdr-radio/tests/staging.rs` (one function per `should_panic` case with its exact message, N2). Mutation rows: re-anchor what your edit breaks (same program change, same Provider; never move a UHD-only or Mock-only row into shared code; no two rows identical; every `new` compiles), add the plan's new rows E2B-xx, each naming its killer in `filter`. You cannot run rows; run only the rows you add or touch, through `remote_mutate.py <list> sim03:4 <ids>`.
- Trace comparison (plan §3.3): the out-of-tree device-call + records + events dump, base `9c84502` vs this tree, over `cargo test -p ezsdr-radio-uhd --lib` — zero differing files on the stable set; report the set size and unstable tests. Copies under /tmp, `CARGO_INCREMENTAL=0`, target outside the tree, deleted afterwards.
- `design/` unchanged; list the note 49 wording amendments (plan §6) in the report, do not edit plan/ files other than the test plan's own "Changes in E2b-2" section.

## Rules

- Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). New timing-dependent tests: 20 runs.
- Builds/tests via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`; mutation rows only via `remote_mutate.py`; never a mutant in the tree; temp files on /tmp; kill by PID. Wait in the foreground for every run.
- Do not add mechanisms the plan does not name; if the plan cannot be built as written, stop that part and say why.
- `cargo fmt --all`; `rdev -- cargo test --workspace` green; `rdev -- cargo clippy --workspace --all-targets` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 9c84502` = 0. No commit.

## Report

`git diff --stat`; the trace comparison result; per new row its killer and outcome; the existing rows re-anchored; any test edited and why; plan points you could not follow.

