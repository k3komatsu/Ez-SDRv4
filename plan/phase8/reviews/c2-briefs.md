# Note 49 C2 — briefs (2026-11-12)

## brief-plan

### Brief — note 49 stage C2, phase 1: the test plan (no code)

Work tree: **/home/komatsu/local_work/Ez-SDRv4-e3** (git worktree, branch `work/c2` from `main` after E3b). Read only there; write only the plan file below. You are the C2 implementer (OpenCode Sonnet 5.5 #high); C2 is **risky** (a lifecycle and the record's outcome; it changes behaviour on both Providers and a Manifest value): the test plan comes before code and Sol reviews it.

Read first: AGENTS.md (§3, §6, §7, §9 in full); plan/phase8/49-time-and-booking-inventory.md (§5.4, especially §5.4.5 and its table, §5.4.8, §5.4.11 the oracle, §11.10 — the owner's decision of 2026-11-08 —, §12's C2 row, the Changes table); KNOWN_LIMITATIONS.md KL-42 (the defect C2 fixes), KL-36, KL-39, KL-41; design/07-radio-model.md RM-37, design/18-uhd-radio.md UR-38, design/09-mock-radio.md MR-42 and MR-25; the E2a, E3a and E3b records (plan/phase8/reviews/e2a-dispositions.md, e3a-dispositions.md, e3b-dispositions.md, e3b-test-plan.md: the book now owns the device-burst state, `DeviceChain`, and the front arithmetic is `segment.rs`). Code: `crates/ezsdr-radio/src/staging.rs` (`TxBook`, `SwitchOutcome`, `Settled`, how switches are settled on a send, a Stop, an abandon, a loss, `Provider::stop`), `crates/ezsdr-radio-uhd/src/provider/tx.rs`, `crates/ezsdr-mock-radio/src/lib.rs` (its switch rows and Stop model), the Manifest `switches` section and its schema (`schemas/`; find where `switches` rows are written and validated), the O1 oracle `crates/ezsdr-acceptance/tests/oracle`, `crates/ezsdr-acceptance/tests/parity.rs`, the Python client if it reads a switch outcome. Also plan/phase8/51-test-timing.md §3 (no test verdict may depend on when a thread runs; `UhdRig` registers its thread on the manual clock since note 51 stage 1b).

## What C2 is (owner, 2026-11-08, note 49 §11.10)

A switch's outcome follows whether the sample at its boundary was sent: **`applied`** only when the operation that carries the boundary sample was sent and the device time passed the boundary; **`cancelled`** when a Stop closed before the boundary or a send went short before it; **`uncertain`** (a new value) when the range that carries the boundary ended in an exception. Both Providers at once (the Mock's Stop row changes with it), the schema, RM-37, UR-38, MR-42; KL-42 is resolved (its entry updated in the same commit as the fix).

## The plan (write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/test-plan.md)

1. **Inventory**: every place a switch is settled today on each Provider (path:line), the outcome each writes, and the sequence; which of them change under C2 and to what. Every reader of the outcome (Manifest, schema, Python, oracle, tests, hardware tests' expectations).
2. **Sequences → expected outcome** (each: the operation sequence, today's outcome on UHD and Mock, C2's outcome, the evidence): a switch whose boundary is sent; a Stop before `b` with `b` before/after the Stop's bound `B` (KL-42's case); a short send before/at/after `b`; an exception in the send carrying `b`; a device loss; `Provider::stop`; an abandon; a switch moved to the next boundary (SendAsap late); two switches. Mark hardware behaviour VERIFIED/INFERRED.
3. **Tests**: for each sequence the test, what it asserts, how it observes it, the wrong behaviour that fails it; the oracle's rows; parity tests where both Providers run the same sequence; existing tests whose expectation changes (a table: test, old, new, why).
4. **Mutation rows** (Rust token anchors) for the new rule, with killers that fail on assertions; existing rows to retarget.
5. **Schema and text**: the schema change (`uncertain`), its regeneration and `schema_freeze`; RM-37, UR-38, MR-42 wording; KL-42; any other spec or KL that describes a switch outcome. **No version bumps and no SCHEMA_CHANGELOG entry** (AGENTS §6).
6. **Hardware tests** whose expectations come from these rules: the AGENTS §9 audit table, and the FakeDevice rehearsals.
7. **Anything user-visible the owner has not decided** (for example how `uncertain` shows in the Python client, or a sequence where the decision's three values do not say which applies): list each as a question for the orchestrator, with a recommendation; do not decide it.
8. **Sub-steps**, each leaving the tree green.

Rules: no code edits in the tree; probes only in a copy outside the repository built locally (`rsync -a --no-times --exclude .cargo` to /tmp/opencode/<name>, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`), not on sim02/sim03; no commit. Final message: a summary, the open questions, and anything in note 49, the specs or KL the code contradicts.

## brief-impl1

### Brief — C2-1: amend the plan for Sol's review and the decisions, then sub-step C2-1

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/c2` at `fbde789` (the step's start). You are the C2 implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/: brief-plan.md, test-plan.md (yours), plan-decisions.md (the owner's answers to Q1, Q2, Q4, Q5 and the orchestrator's to Q3, Q6, Q7), sol-plan.md (Sol's plan review: FAIL, one major, three minors; all adopted, decided by the orchestrator).

## Amend the plan first (a section "Changes after Sol's review and the decisions" at the top of test-plan.md)

- **M1:** a loss during the unresolved boundary send: issue the send carrying `b` → loss at `t < b` → the send completes successfully → the row must read `cancelled` (Q1), not `applied`. The deferred settlement must keep the loss sample (or the loss's settlement) and apply it at completion; add the sequence to §2, its tests (UHD and the Mock's analogue if reachable), and a mutation row whose killer fails on an assertion. Also the loss arriving after the completion but before device time reaches `b`, and a Stop and a loss both during one boundary send.
- **m2:** two accepted switches: a test that each settles on its own boundary send (outcome isolation).
- **m3:** H4 (`test-plan.md:57`) cites BurstAck timing, not an underflow's timestamp: mark that claim INFERRED or give the matching evidence.
- **m4:** add `switch()`'s docstring (`python/ezsdr/session.py:498-504`, Q2: the outcome is one of `applied`, `cancelled`, `uncertain` in the Manifest's `switches`) to §4 and to the sub-step that changes RM-37/UR-38.
- Fold Q1 (C2-3) into the sequences so that C2-2's deferred settlement and C2-3's loss sample agree; if M1 needs the loss sample in C2-2, move it there and say so.

## Then: sub-step C2-1 (plan §5): the oracle functions and their self-tests; no production change

## Rules

Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). **Build and test locally** in this worktree (its own `.cargo/config.toml` target), not on sim02/sim03. Run the rows you add or touch through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` and wait for it to finish before reporting. Temp files on /tmp; kill by PID only; never a mutant in the tree; probes only in a copy outside the repository. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base fbde789` = 0. No commit.

## Report

`git diff --stat`; the plan amendments; per test: what it pins, runs; contradictions with the plan.

## brief-impl2

### Brief — C2-2 (see test-plan.md §5, row C2-2)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/c2` (step start `fbde789`; earlier sub-steps are local commits — `git log fbde789..HEAD`). You are the C2 implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/: brief-plan.md, test-plan.md (all its "Changes" sections), plan-decisions.md, sol-plan.md, brief-impl1.md and the previous sub-steps' briefs. Do **only row C2-2** of plan §5 as amended, exactly as the row states (content, tests, rows, text).

Rules: behaviour changes only as the plan states for this row; tests whose expectation changes are only those the plan's table names (any other is reported, not edited). Spec and KL text in the same commit as the behaviour (AGENTS §2). No version bumps and no SCHEMA_CHANGELOG entry. Mutation rows: re-anchor what your edit breaks; run the rows you add or touch through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` and **wait for it to finish before reporting**. Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). **Build and test locally**, not on sim02/sim03. Never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; the Python suites when the row touches `python/` (`PYTHONDONTWRITEBYTECODE=1`, venv `~/.cache/ezsdr-venv`, `EZSDR_SERVER` from this tree's build); `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base fbde789` = 0. Add a "Changes in C2-2" section to test-plan.md. No commit.

Report: `git diff --stat`; what the row required and what you did; rows and outcomes; deviations with reasons.

## brief-impl34

### Brief — C2-3 + C2-4 (see test-plan.md §5, rows C2-3 and C2-4)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/c2` (step start `fbde789`; earlier sub-steps are local commits — `git log fbde789..HEAD`). You are the C2 implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/: brief-plan.md, test-plan.md (all its "Changes" sections), plan-decisions.md, sol-plan.md, brief-impl1.md and the previous sub-steps' briefs. Do **rows C2-3 and C2-4 together** of plan §5 as amended, exactly as the row states (content, tests, rows, text).

Rules: behaviour changes only as the plan states for this row; tests whose expectation changes are only those the plan's table names (any other is reported, not edited). Spec and KL text in the same commit as the behaviour (AGENTS §2). No version bumps and no SCHEMA_CHANGELOG entry. Mutation rows: re-anchor what your edit breaks; run the rows you add or touch through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` and **wait for it to finish before reporting**. Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). **Build and test locally**, not on sim02/sim03. Never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; the Python suites when the row touches `python/` (`PYTHONDONTWRITEBYTECODE=1`, venv `~/.cache/ezsdr-venv`, `EZSDR_SERVER` from this tree's build); `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base fbde789` = 0. Add a "Changes in C2-4" section to test-plan.md. No commit.

Report: `git diff --stat`; what the row required and what you did; rows and outcomes; deviations with reasons.

## Orchestrator decisions after C2-2 (2026-11-12)
- C2-3 is merged into this run (plan §5: "if C2-2 leaves nothing, C2-3 is empty and is merged into C2-4"). C2-2 already moved the loss sample (`stop_at(.., lost)`) and wrote M2. What remains of C2-3: confirm the Mock loss derivation (`cut = t_k`) with the existing M2 test, and confirm `schema_freeze` is unchanged. Write the result in one "Changes in C2-3 + C2-4" section of test-plan.md.
- `rules.py` and the gate need `--renames /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/renames.txt` (C2-2 renamed one `e2a_s11_…` test); pass it in your rules run.
- The `ideal` Mock panic you found (a switch at the repeat's first sample) is filed as #100; leave it alone.

## brief-impl5

### Brief — C2-5 (see test-plan.md §5, row C2-5)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/c2` (step start `fbde789`; earlier sub-steps are local commits — `git log fbde789..HEAD`). You are the C2 implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/: brief-plan.md, test-plan.md (all its "Changes" sections), plan-decisions.md, sol-plan.md, brief-impl1.md and the previous sub-steps' briefs. Do **only row C2-5** of plan §5 as amended, exactly as the row states (content, tests, rows, text).

Rules: behaviour changes only as the plan states for this row; tests whose expectation changes are only those the plan's table names (any other is reported, not edited). Spec and KL text in the same commit as the behaviour (AGENTS §2). No version bumps and no SCHEMA_CHANGELOG entry. Mutation rows: re-anchor what your edit breaks; run the rows you add or touch through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` and **wait for it to finish before reporting**. Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). **Build and test locally**, not on sim02/sim03. Never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; the Python suites when the row touches `python/` (`PYTHONDONTWRITEBYTECODE=1`, venv `~/.cache/ezsdr-venv`, `EZSDR_SERVER` from this tree's build); `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base fbde789` = 0. Add a "Changes in C2-5" section to test-plan.md. No commit.

Report: `git diff --stat`; what the row required and what you did; rows and outcomes; deviations with reasons.

## Orchestrator notes after C2-3 + C2-4 (2026-11-12)
- C2-3 and C2-4 are committed as one local commit. `rules.py` needs `--renames /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/renames.txt`.
- N47S5-152 and E2A2B-06 survived after C2-2 (the abandon's rewrite of settled `applied` rows, now unreachable). They go with the code in this row: retire them with the six rows the plan names, or retarget if the behaviour stays reachable; no `known-survivors.txt` entry.
- `n47s5_front_rehearsal_on_the_fake` failed once under full-workspace load in C2-4 (a real-clock Run-based rehearsal, note 51 tables row "late L → fault", stage 2c). If it fails again, capture the message and report it; do not edit it.

## brief-fix1

### Brief — C2 fix round 1 (after Sol's code review)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/c2` at `e2b1810` (step start `fbde789`; `git log fbde789..HEAD`). You are the C2 implementer (OpenCode Sonnet 5.5 #high). Read /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/sol-code.md (Sol's code review: FAIL, two majors, one minor), test-plan.md with all its "Changes" sections, and the previous briefs. All three findings are **adopted** (decided by the orchestrator). For each major, write the failing test first (deterministic: frozen rig / record or book level, no dependence on when a thread runs, plan/phase8/51-test-timing.md §3), see it fail on `e2b1810`, then fix.

1. **Major 1 — a loss after `Provider::stop` loses the playback cutoff.** `Streams::end` (`crates/ezsdr-radio-uhd/src/provider/core.rs` ~644) returns early once `ended` is set, so a loss after a `Provider::stop` never reaches `stop_at(.., lost)` and an in-flight boundary send that then completes whole settles `applied` from the stored `played = i64::MAX`. Sequence: issue the boundary send at `b`; `Provider::stop` with `B > b` (row pending); loss at `t_k < b`; the send completes whole → must be `cancelled` (Q1: a loss settles by device time; `played = min`). Fix: a loss after the streams already ended still records its loss sample on any unresolved switch (the `Halted.played = min(..)` path), without re-ending streams or re-booking ends. Test: the Provider-Stop-then-loss cell on the UHD frozen rig (`tx/tests/c2.rs`, beside U15, which uses a Control Stop), plus the reverse order if it is a different path.

2. **Major 2 — a boundary send whose result never comes leaves the final Manifest `pending`.** If the TX worker panics after issuing the boundary send (panic handler `provider/mod.rs` ~375 reports a loss; no `complete`), or is left detached by `join` (UR-16), the switch stays `InFlight` and `write_sections` (`core.settle_switches()`) writes `pending`. Rule (decided by the orchestrator): **when the Manifest is written, an unresolved boundary send is settled as a send whose result is unknown** — the same settlement as `Outcome::Failed` (`uncertain`, unless a Stop's bound `B ≤ b` cancels it first, the oracle's order). One place, at the Manifest write (or at the end of the owner's thread, if that is the single place both paths pass — say which and why), not a second rule per path. Tests: the worker-panic path (deterministic: a scripted panic at the boundary send's hook, if the test support has one; otherwise add the smallest hook) → Manifest `uncertain`; a record/book-level test of the cleanup settlement. Add the oracle cell if the oracle models it.

3. **Minor 3 — the specs omit the immediate cancellation.** C2-2's deviation (a halt with `B ≤ b` cancels at once even while the boundary send is in flight) is **adopted**: it is the oracle's order (a Stop at or before `b` cancels whatever the send did). Make `design/07-radio-model.md` (~357, RM-37), `design/18-uhd-radio.md` (~305, UR-38) and `plan/phase8/49-time-and-booking-inventory.md` (~256) say so in one clause each, and state major 2's rule (an unresolved boundary send at the Manifest write is `uncertain`) where the specs list the outcomes. Keep RM-37/UR-38 normative text short (AGENTS §2 format: no rationale or history in the rule).

Rules: behaviour changes only as above. Mutation rows for each fix (Rust token anchors), killer on an assertion; run the new rows and the rows whose code or killing test you touched through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` and **wait for it to finish before reporting**. New tests whose verdict can depend on timing: 20 runs; pure ones once. **Build and test locally**, not on sim02/sim03. Never a mutant in the tree; kill by PID only. `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base fbde789 --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/renames.txt` = 0. Add a "Changes in fix round 1" section to test-plan.md. No commit.

Report: `git diff --stat`; per finding: the failing test (name, the failure on `e2b1810`), the fix, rows and outcomes; deviations with reasons.

## brief-fix2

### Brief — C2 fix round 2 (after the Opus re-check of fix round 1)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/c2` at `9f6b853` (step start `fbde789`). You are the C2 implementer (OpenCode Sonnet 5.5 #high). Read brief-fix1.md, the end of fix1.log, test-plan.md's "Changes in fix round 1", and this brief (all in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/). The Opus re-check closed both majors of Sol's review and found the items below; all adopted (decided by the orchestrator).

1. **A Stop's `applied` is overturned by a later loss before `b` (decided by the orchestrator, applying the owner's Q1: a loss settles by device time).** Today: boundary send returned whole (`carried = Sent`), device time `< b`; a Stop or `Provider::stop` with `B > b` settles `applied` at once (staging.rs ~1223, ~1298); a loss at `t_k ≤ b` after it is a no-op (`lose()` finds nothing pending) → the row says `applied` though the device never played `b`. Loss first gives `cancelled`, and the in-flight form of the same history gives `cancelled` (`c2_s22`); the oracle (`o1_oracle.rs` ~223) already says `Cancelled` for Whole + Stop + a loss before `b`. Fix: a halt with `B > b` on a `Sent` switch whose `b` device time has not reached keeps the row pending with the halt recorded (the existing `Halted { stop, played }`), and it settles: `applied` when device time reaches `b` (`advance`, with the first halt's geometry), `cancelled` when a loss lowers `played` to `≤ b` (`lose`), and `applied` at the Manifest write if still unresolved (`settle_unresolved`: the send was taken and nothing says the device did not play `b`). If device time is already `≥ b` at the halt, it settles `applied` at once as today. Keep one settlement function (`settle_halted`); no second rule per path. Tests first, failing on `9f6b853`: record level (Sent + halt `B > b` + loss `t_k ≤ b` → `cancelled`; Sent + halt + advance past `b` → `applied`; Sent + halt + Manifest write → `applied`), UHD frozen rig (`tx/tests/c2.rs`: boundary send whole, `Provider::stop` with `B > b`, then a loss before `b` → Manifest `cancelled`; and a Control Stop variant), both through the oracle functions where the oracle has them. Rows for the new branch(es).
2. **RM-37 wording (minor):** say which wins between "a loss at or before `b` → cancelled" and "an exception → uncertain": the exception wins (code: `Unknown` checked before `played`, staging.rs ~1297; UR-38 already says so). One clause. Also state item 1 where RM-37 and UR-38 say a Stop with `B > b` after the boundary send gives `applied`: unless a loss at a sample at or before `b` follows before device time reaches `b`.
3. **Note 49 §5.4.5 row (minor):** replace the self-contradicting "結果に関わらず cancelled（Failed は uncertain）" and "B ≤ b の Stop と loss" with: "loss の sample が b 以前なら cancelled（Failed は uncertain）; Stop または loss の bound が b 以前ならその場で cancelled", plus item 1 in one sentence.
4. **Oracle (minor):** `switch_outcome_after_loss` takes no bound; the `provider/mod.rs` panic test's third case asserts the literal `"cancelled"`. Either give the oracle the bound (so the cancel-at-once case is expressible) and use it in that test, or say in the test why the literal is right — prefer the oracle.
5. Recorded, no change: the detached-join path (UR-16) reaches `settle_unresolved` but has no test; C2-24 (minimum vs overwrite) is equivalent in production (one loss is published) and is a staging-level contract row. Do not change these.

Rules: as brief-fix1.md (local build and tests; failing tests first; rows run through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` for the new rows and every row whose code or killing test you touched, and wait for it; new timing-dependent tests 20 runs; `cargo fmt --all`; `cargo test --workspace`; clippy `-D warnings`; `rules.py --base fbde789 --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/renames.txt` = 0). Add a "Changes in fix round 2" section to test-plan.md. No commit.

Report: `git diff --stat`; per item: failing test (name, failure on `9f6b853`), fix, rows and outcomes; deviations with reasons.

## brief-fix3

### Brief — C2 fix round 3 (after the Opus re-check of fix round 2)

Work tree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/c2` at the fix-round-2 commit (HEAD; step start `fbde789`). You are the C2 implementer (OpenCode Sonnet 5.5 #high). Read brief-fix2.md, the end of fix2.log, test-plan.md's "Changes in fix round 2", and recheck2.md (the Opus re-check of round 2), all in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/. **Round 2's mutation run never ran** (it was queued behind a gate and cancelled before it copied the tree): its row list is in fix2-ids.txt; run those rows with this round's at the end.

Adopted (decided by the orchestrator):
1. **An abandon must halt at device time, not at its cursor.** `Bookings::abandon` halts with `at = cursor_k` (staging.rs ~1360), but `Halted.at` is device time. Reachable: boundary send whole at device time `< b`; the next send returns a non-lost error (tx.rs ~608/666) or a sequence-error report (tx.rs ~1136) → abandon at a cursor past `b` → `applied` at once; a loss at a sample ≤ b then is a no-op (oracle: `cancelled`). Same if a Stop left the row waiting and an abandon follows (`first.at.max(cursor)` ends the wait). Fix: pass device time into `abandon` and use it as the halt's `at`, keeping the chain cut at the cursor; `TxBook::abandon` still clears `playing` when the switch stays pending. Failing test first (record level, and a UHD frozen-rig cell with a non-lost send error after the boundary send, then a loss before `b` → `cancelled`; and the advance-past-`b` → `applied` cell).
2. **F8 on a row a Stop is holding** (failed reopen, staging.rs ~858) now gives `cancelled` (was `applied` at once). This matches UR-38 and `c2_s15`; add a test and say it in RM-37's clause.
3. **`settle_unresolved` writes `Applied` without `apply()`**: make its Sent branch call `apply(switch, halted.stop, Applied)` so one settlement keeps the record's chains (the Mock runs it mid-Run).
4. **Text:** RM-37 (design/07 ~358) and UR-38 (design/18 ~304) still say a Stop in flight "settles it by" the result — a whole result before device time reaches `b` keeps the row waiting; say so. RM-37's "(the Stop leaves the switch pending until then)" applies to UHD; MR-42 (design/09) says the Mock settles at the Stop by its payload cut (no loss can follow a Mock Stop) — make RM-37 not claim it for every Provider and add the clause to MR-42.
5. Recorded, no change: the tie at exactly device time `b` (a Stop or `advance` at sample `b` then a loss at sample `b` → `applied`; loss first → `cancelled`; from `advance`'s `b <= now`, predates C2); C2-32 and C2-38 near-duplicate (keep both); the detached-join path has no test.

Rules: as brief-fix1.md (failing tests first; local build and tests; `cargo fmt --all`; `cargo test --workspace`; clippy `-D warnings`; `rules.py --base fbde789 --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/renames.txt` = 0; new timing-dependent tests 20 runs). Rows: fix2-ids.txt plus this round's new and touched rows in **one** run: `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids> > /home/komatsu/.cache/ezsdr-fixes/2026-11-12-c2/fix3-mut.log 2>&1` — the sims may be busy with another lane's gate (simq waits; that can take 30+ minutes). **Do not end your run until that command has finished and you have read its result**; poll its PID if your tool call times out. Add a "Changes in fix round 3" section to test-plan.md. No commit.

Report: `git diff --stat`; per item the failing test, fix, rows and outcomes; deviations with reasons.

## Decisions on the test plan's questions

### C2 — decisions on the test plan's questions (2026-11-12)

Owner (2026-11-12):
- Q1 = a loss settles by device time at the loss: `applied` iff device time had passed `b`, `uncertain` if the boundary send failed, else `cancelled` (not as a Stop).
- Q2 = A: `uncertain` appears in the Manifest only (readable from Python through `sdr.manifest`); nothing new in the Python client; `switch()`'s docstring says the outcome is one of `applied`, `cancelled`, `uncertain` in the Manifest's `switches`; the hardware verdict (`iii::switch_verdict`) treats `uncertain` as Inconclusive. A way to read Run records during a Session, if ever needed, is a general feature, not switch-specific.
- Q4 = accept the new refusal text (`UR-21: … no pending switch`) for a second switch on an unissued one past its boundary; one clause in KL-41.
- Q5 = accept the Mock/UHD difference near the window edge (each matches what it emitted) and state it in MR-42, as MR-25 does for the cut.

Decided by the orchestrator:
- Q3: a Stop while the boundary send is in flight settles by that send's result; the row stays `pending` until the completion (`Switch.halted`).
- Q6: KL-42 becomes "Gone in C2" with its symptom corrected (the Mock had the same defect when `b` falls between its payload cut and `B`); the entry keeps its number.
- Q7: `Bookings::switch(.., carried)` explicit.
