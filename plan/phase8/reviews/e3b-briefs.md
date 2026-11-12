# E3b — briefs (2026-11-12)

## brief-plan

## Brief — note 49 stage E3b, phase 1: the test plan (no code)

Work tree: **/home/komatsu/local_work/Ez-SDRv4-e3** (git worktree, branch `work/e3b` from `main` `0287fce`). Read only there; write only the plan file below. You are the E3b implementer (OpenCode Sonnet 5.5 #high); this phase is the test plan only.

Read first: AGENTS.md (§3, §7, §9 in full — E3b is **risky**: it moves device-burst state, a lifecycle, into the book and changes what receipt reads; the test plan comes before code and Sol reviews it); plan/phase8/49-time-and-booking-inventory.md (§5.4, §12's E3 row and the paragraph after the table); plan/phase8/reviews/e2a-sketch.md (§2 item 2, P2/P3, W5, the "E3 must move TxChain and the segmentation together" line, and what it says stays until E3: `front`, `may_be_handed`, the stat `tx_close_past_record_front`); the E3a records plan/phase8/reviews/e3a-dispositions.md, e3a-test-plan.md (E3a extracted `crates/ezsdr-radio/src/segment.rs`; E3b is the part E3a deferred: decision K1 in e3a-briefs.md "Orchestrator decisions"); e2a-dispositions.md, e2b-dispositions.md; design/07-radio-model.md RM-15, RM-16, RM-21; design/18-uhd-radio.md UR-22, UR-23, UR-33, UR-38; design/09-mock-radio.md MR-25. Code: `crates/ezsdr-radio/src/staging.rs` (`TxBook`, the record's `front()` prediction, `may_be_handed`), `crates/ezsdr-radio/src/segment.rs`, `crates/ezsdr-radio-uhd/src/provider/tx.rs` (`TxChain`: the device burst, its next sample, the held tail, the front, recovery; `close_device_burst`, `end_open`, `continuation_open`), `crates/ezsdr-mock-radio/src/lib.rs`, the O1 oracle `crates/ezsdr-acceptance/tests/oracle`, `crates/ezsdr-acceptance/tests/parity.rs`. Also plan/phase8/51-test-timing.md §3 (no test verdict may depend on when a thread runs; `UhdRig` registers its thread on the manual clock only from note 51 stage 1b, which is not on `main` yet — use the frozen/stepped rigs and FakeDevice rehearsals E3a used).

### What E3b is

Move `TxChain` (device-burst state) into the book, so the state machine owns it, and replace the record's front prediction (`front()`) and `may_be_handed` with answers computed from that state through `segment.rs`, so there is one front arithmetic (D′). Remove the stat `tx_close_past_record_front` if nothing needs it then. **Intended: no behaviour change.** If the prediction and the actual state can disagree today (a receipt answer, a front, a refusal that would change), that is a finding: report each such sequence with its observable difference (the Manifest, a refusal, a FakeDevice call) — do not choose; the orchestrator decides, and a user-visible change goes to the owner.

### The plan (write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3b/test-plan.md)

1. **Inventory**: each piece of `TxChain` state and each reader/writer (path:line); each use of `front()`, `may_be_handed` and `tx_close_past_record_front`; where prediction and state can disagree today (sequences), and the tests that pin each.
2. **Characterization first**: the observable behaviour to keep (FakeDevice `calls()`, Manifest rows, receipt answers and refusals, Mock blocks), and tests green on `0287fce` before any production change, with full ordered snapshots where a refactor could shift them.
3. **Sequences → tests** (each: the test, what it asserts, how it observes it, the wrong behaviour that fails it): pure transition tables for the book's new device-burst state; receipt answers at a front (Stop, a booking at the front ±1, after a short closed burst, after recovery, during configure/reopen); UHD wiring and order; parity where both Providers run the same booking.
4. **Mutation rows**: new rows (Rust token anchors) and existing rows to retarget or re-anchor (grep the anchors of the code you will move), with an old row → new anchor/killer table; leftovers removed.
5. **Hardware tests** whose expectations come from these rules: the AGENTS §9 audit table, and the FakeDevice rehearsals.
6. **Text**: `design/` rules and KNOWN_LIMITATIONS entries that describe the front, receipt or the stat, and whether wording must change.
7. **Sub-steps**: a split with the tree green after each (characterization first).

Rules: no code edits in the tree; probes only in a copy outside the repository (`rsync -a --no-times --exclude .cargo` to /tmp/opencode/<name>, then build there with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, **locally**, not on sim02/sim03, which run gates and mutation lists); no commit. Final message: a summary of the plan, the sub-steps, the prediction/state disagreements found, and anything in note 49, the E2a sketch or the E3a records the code contradicts.

## brief-impl1

## Brief — E3b-1: amend the plan for Sol's review and the decisions, then the characterization tests (test code only)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/e3b` at `0287fce` (the step's start). You are the E3b implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3b/: brief-plan.md, test-plan.md (yours), plan-decisions.md (K1 = S by the owner; **the stat `tx_close_past_record_front` is kept** — remove every stat-deletion instruction from the plan: E3b-4 no longer removes it, its row, its test or UR-23's test name; K3–K7 adopted), and sol-plan.md (Sol's plan review: PASS WITH CHANGES; all findings adopted, decided by the orchestrator).

### Amend the plan first (a section "Changes after Sol's review and the decisions" at the top of test-plan.md)

- **M1:** recovery, reopen, T3 flush and shutdown wait on the device's host clock (`provider/core.rs:1209-1234`), which `Frozen` stops (`provider/test_support.rs:136-157`). For every clock-waiting operation in C1/C3/C4 and their mutation killers, use the existing `released` participant helper (`provider/test_support.rs:170-184`): it returns the owner and a new freeze before the snapshot. Never drop the freeze around an unregistered test thread.
- **m1:** C3/C4 counts cannot see a write moved between two sections: add ordered snapshots at the section boundaries (booking completion vs the chain effect; before close vs the close's result; the flush front vs reopen completion), assert each seam fired and receipt still reads the prediction; where folding in E3b-4 intentionally changes an intermediate state, state the permitted difference.
- **m2:** P-MH's "first send fits ⇒ may have handed" needs "no preceding open join" (`segment.rs:135-147` vs `staging.rs:1103-1107`); pin that counterexample separately.
- **m3:** receipt's front uses checked addition (`staging.rs:1128-1133`), the executor's saturating (`tx.rs:826,866,942,1026`): state both in the shared functions' contract, with boundary rows for every close shape including the flush conversion; a receipt overflow stays a refusal.
- K5: complete the search of every `Tx::new` caller (the plan says INFERRED) and record it.

### Then: E3b-1 (plan §7, test code only)

The characterization tests C1–C6 (with C6 pinning F1 and F4 as they are; F4 is issue #99 — cite it), P-tables that can run on today's code, the receipt table on both Providers, lock-section counts and the ordered snapshots above — **each green on today's code `0287fce`**. Run each twice; a field that differs between runs is named and excluded with its reason; tests whose verdict could depend on a thread's timing 20 times. No production change and no mutation rows yet (they come with the code). A planned expectation today's code contradicts is reported, not fixed.

### Rules

Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). **Build and test locally** in this worktree (its own `.cargo/config.toml` target), not on sim02/sim03 (they run gates and mutation lists): `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`. Temp files on /tmp; kill by PID only; never a mutant in the tree; probes only in a copy outside the repository. `cargo fmt --all`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 0287fce` = 0. No commit.

### Report

`git diff --stat`; per test: green ×2 (×20 where timing-dependent), what it pins, exclusions; the plan amendments; contradictions with the plan.

## brief-impl2

## Brief — E3b-2 (see test-plan.md §7, row E3b-2)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/e3b` (step start `0287fce`; earlier sub-steps are local commits — `git log 0287fce..HEAD`). You are the E3b implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3b/: brief-plan.md, test-plan.md (all its "Changes" sections), plan-decisions.md (shape S: receipt keeps reading the record; **the stat `tx_close_past_record_front` stays**), sol-plan.md, brief-impl1.md and the previous sub-steps' briefs. Do **only row E3b-2** of plan §7, exactly as the row states (its content, its rows, its review checklist), and the plan's trace comparison where the row names it.

Rules: no behaviour change; every E3b-1 characterization test (C1–C6, C3b, P-CF, P-MH, the receipt table) stays green **with its expectations unedited** (a test that reads the chain through `dev_*`/`poke_*` may have those helpers' bodies changed, not its assertions). Mutation rows: re-anchor what your edit breaks (same program change; no two rows identical; every `new` compiles); run the rows you add or touch through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` (`simq` waits while another lane runs a gate or full lists). Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). **Build and test locally** in this worktree (its own `.cargo/config.toml` target), not on sim02/sim03; trace-comparison copies under /tmp/opencode with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, deleted after; never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 0287fce` = 0. Add a "Changes in E3b-2" section to test-plan.md for anything that differs from the plan. No commit.

Report: `git diff --stat`; what the row required and what you did; rows and outcomes; the trace comparison result if the row names it; deviations with reasons.

## brief-impl3

## Brief — E3b-3 (see test-plan.md §7, row E3b-3)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/e3b` (step start `0287fce`; earlier sub-steps are local commits — `git log 0287fce..HEAD`). You are the E3b implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3b/: brief-plan.md, test-plan.md (all its "Changes" sections), plan-decisions.md (shape S: receipt keeps reading the record; **the stat `tx_close_past_record_front` stays**), sol-plan.md, brief-impl1.md and the previous sub-steps' briefs. Do **only row E3b-3** of plan §7, exactly as the row states (its content, its rows, its review checklist), and the plan's trace comparison where the row names it.

Rules: no behaviour change; every E3b-1 characterization test (C1–C6, C3b, P-CF, P-MH, the receipt table) stays green **with its expectations unedited** (a test that reads the chain through `dev_*`/`poke_*` may have those helpers' bodies changed, not its assertions). Mutation rows: re-anchor what your edit breaks (same program change; no two rows identical; every `new` compiles); run the rows you add or touch through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` (`simq` waits while another lane runs a gate or full lists). Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). **Build and test locally** in this worktree (its own `.cargo/config.toml` target), not on sim02/sim03; trace-comparison copies under /tmp/opencode with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, deleted after; never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 0287fce` = 0. Add a "Changes in E3b-3" section to test-plan.md for anything that differs from the plan. No commit.

E3b-2 (local commit `86a4fae`) ended before writing its "Changes in E3b-2" section: write it too, from `git show 86a4fae` (its 42 rows all killed: e3b2-mut.log in the directory above). Wait for every row run you start to finish before reporting.

Report: `git diff --stat`; what the row required and what you did; rows and outcomes; the trace comparison result if the row names it; deviations with reasons.

## brief-impl4

## Brief — E3b-4 (see test-plan.md §7, row E3b-4)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/e3b` (step start `0287fce`; earlier sub-steps are local commits — `git log 0287fce..HEAD`). You are the E3b implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-e3b/: brief-plan.md, test-plan.md (all its "Changes" sections), plan-decisions.md (shape S: receipt keeps reading the record; **the stat `tx_close_past_record_front` stays**), sol-plan.md, brief-impl1.md and the previous sub-steps' briefs. Do **only row E3b-4** of plan §7, exactly as the row states (its content, its rows, its review checklist), and the plan's trace comparison where the row names it.

Rules: no behaviour change; every E3b-1 characterization test (C1–C6, C3b, P-CF, P-MH, the receipt table) stays green **with its expectations unedited** (a test that reads the chain through `dev_*`/`poke_*` may have those helpers' bodies changed, not its assertions). Mutation rows: re-anchor what your edit breaks (same program change; no two rows identical; every `new` compiles); run the rows you add or touch through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` (`simq` waits while another lane runs a gate or full lists). Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). **Build and test locally** in this worktree (its own `.cargo/config.toml` target), not on sim02/sim03; trace-comparison copies under /tmp/opencode with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, deleted after; never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 0287fce` = 0. Add a "Changes in E3b-4" section to test-plan.md for anything that differs from the plan. No commit.

Also (decided by the orchestrator): run the plan's **trace comparison** once here, base `0287fce` against this sub-step's head (it covers E3b-2…4); and C4's section counts must return to the base counts after the fold — E3b-3 left one count it did not trace (the joined piece, 12 → 18): explain every count that stays above the base. Wait for every row run you start to finish before reporting.

Report: `git diff --stat`; what the row required and what you did; rows and outcomes; the trace comparison result if the row names it; deviations with reasons.

## Decisions on the test plan

## E3b — decisions on the test plan's open points (2026-11-12)

- K1 shape: **S** (owner, 2026-11-12: 「E3bについてSでお願いします」). The device-burst state moves into the book and the front arithmetic is one set of functions in `segment.rs`; receipt keeps reading the record (the prediction). No behaviour change. L (receipt reads the executor's state: F1, F2, F4, F5, F7 change user-visible receipt answers) is not part of E3b; each disagreement is decided separately later, with a bench plan where it relies on hardware.
- K2 the stat `tx_close_past_record_front`: **kept** in E3b (decided by the orchestrator). Removing it takes a key out of the Manifest's `stats`, which is the owner's to decide; E3b stays a no-change step. Its removal goes to the Phase 8-end review with the evidence that it never fires (test plan §1.3). Drop E3b-4's removal of the stat, its row, test and the UR-23 rename.
- K3 the Mock does not drive the chain (`DeviceModel`, MR-24, stays an independent check): adopted.
- K4 the tail payload as a type parameter of the book: adopted.
- K5 `Tx::new` resets the book's chain: adopted; the brief's search of every `Tx::new` caller must be completed (the plan says INFERRED).
- K6 F4: filed as #99 (mech:planned-state-read-as-actual) by the orchestrator; no fix in E3b; C6 pins today's answer. A KL entry is not added now (a defect with an issue is not listed in KNOWN_LIMITATIONS).
- K7 sub-steps: adopted (characterization first); E3b-3 and E3b-4 may merge if the section counts stay equal.
- X1–X5 (records the code contradicts): note 49 §3 R3 and §4 line 138 are corrected by the orchestrator at landing.
