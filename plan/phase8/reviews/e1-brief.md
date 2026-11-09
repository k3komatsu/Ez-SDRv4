# Brief — E1: bookings and Stops in one receipt-ordered list (note 49 §5.4.4 item 6, §12 row E1, §12.1)

Repository: /home/komatsu/local_work/Ez-SDRv4, branch `main`, base commit `4151197` (O1). Work in the main tree, uncommitted. Read first: AGENTS.md "Safety core", §3, §7, §9 (you are the implementer); design note `plan/phase8/49-time-and-booking-inventory.md` §4, §5.4.3, §5.4.4, §5.4.9, §11.6, §12, §12.1 (read only; the orchestrator's file); **the shape sketch `e1-sketch.md`** (beside this brief: `/home/komatsu/.cache/ezsdr-fixes/2026-11-10-e1/sketch.md`), which is binding except where this brief overrides it; the rules it touches: `design/07-radio-model.md` RM-15, RM-16, RM-37; `design/18-uhd-radio.md` UR-21–UR-26, UR-38; `design/09-mock-radio.md` MR-15, MR-25, MR-42.

**Risk: risky** (AGENTS §9: a queue and ownership change). Order: **test plan first, no code** → Sol reviews the plan and the sketch → you implement → orchestrator review, full mutation run, Sol's code review. Self-review is waived (Sonnet implementer).

## Goal

Accepted bookings and Stops (and `Provider::stop`'s Run end) go into one receipt-ordered list per transmit stream, under the streams lock beside `Bookings`; the UHD owner thread and the Mock read it in receipt order. `TxCmd::Burst`, `TxCmd::Stop`, `Control.to_tx`, uhd-tx's private `held`, the Mock's `held` and `Tx::command` go; only `TxCmd::Shutdown` stays on the mpsc. Shape, call order and the removal table: the sketch §1–§3.

**No behaviour change**, except the points below, which make the code follow text the spec already has (RM-16) and are named in the record so no reviewer counts them as regressions:
1. A booking cancelled by a `Provider::stop` before the clock's origin gets the `ProviderStop` note (no `COMMAND_REJECTED`) whatever the order of the Run end and the thread's shutdown (sketch Q3; note 49 §5.4.4 6).
2. `Provider::stop` closes the open transmit burst at uhd-tx's next turn (handed cursor ≤ B − 3), not after the uhd-control join (sketch Q8, RM-16 "no later than B").
3. The step reads `now` inside the critical section that checks the list for unread entries, so a turn never hands past a Stop it has not read (sketch Q2, §5 (f); note 49 §4's second window).
4. The uncut-segment check is repeated inside receipt's main critical section, so a booking racing a `Provider::stop` cut is refused at receipt (sketch §5 (d)).

## Orchestrator's decisions on the sketch's open points

- (a) `cancel_unstarted` stays for `clock == None` after a failed reopen (sketch Q9; pinned at `tx.rs:1641-1651`). Only `end_clock`'s cancel loop and the domain-mismatch arm go. `CancelReason::ClockEnded`'s doc: "a failed streamer reopen ended the burst's transmit clock (UR-22)".
- (b) `ur_23_a_clock_end_cancels_every_held_burst` and `ur_25_a_burst_booked_on_an_ended_clock_is_cancelled_when_received` are deleted (unreachable state; give the argument in the plan); `ur_25_an_end_booked_after_the_first_read_cancels_the_held_burst_at_the_origin` is rewritten to the ProviderStop note, and you update its `*Test:*`/text mention at `design/18-uhd-radio.md:543` (report the exact edit).
- (e) The four split-receipt tests (sketch §3): for each, keep the assertion on a reachable path (book atomically, or reach the state through abandon) or delete it with the argument why its failure case is unreachable or covered elsewhere (AGENTS §9 Maintainability 3). Update the `*Test:*` lines that name them (`design/07-radio-model.md:176`, `design/18-uhd-radio.md:266`, and any other: grep).
- N47S5-57: decide reachability of `continuation_open`'s joined clause (`staging.rs:284`) in the plan. If unreachable, remove the clause and the row with the argument; otherwise write a killer.
- #85 is not fixed by E1 (sketch §5): the pin `o1_uhd_t6b2_…_known_divergence_85` keeps its assertion; only its comment's "E1/E2a" becomes "E2a".
- #84 and #86 pins stay unchanged.
- `seq: u64` is not added (sketch §1). `Bookings` is unchanged.

Spec text: you may edit `*Test:*` lines in `design/` that name a test you rename, rewrite or delete, and the `CancelReason::ClockEnded` doc. Any other `design/` sentence the change makes false: list it in the report with the proposed text; the orchestrator writes it.

## Sub-steps

E1a: `TxList` in `ezsdr_radio::staging` (with its pure tests) and the UHD Module. E1b: MockRadio. Each ends green (`cargo test` for the workspace, O1 tests included) and is reported separately; the orchestrator reviews and commits each on `work/e1` before the next starts.

## Test plan (deliver this first, then STOP and report)

Write `/home/komatsu/.cache/ezsdr-fixes/2026-11-10-e1/test-plan.md`:
- for each sketch sequence Q1–Q11 (and any you add): the test (name, file; existing test reused or new), the rig (FakeDevice manual clock or the Mock's stepped run; no wall-clock waits, design note 37: a verdict never depends on when the test thread runs), what it asserts and how it observes it (samples via `FakeDevice::transmitted`/`calls`, Manifest `bursts`/`rejected`/`switches`, events), and the wrong behaviour that fails it — in particular: a Stop read after a booking received after it (receipt order broken), a turn that hands past an unread Stop (Q2), a pre-origin `Provider::stop` cancellation that emits `COMMAND_REJECTED` (Q3), a Run end handled only at Shutdown (Q8), a booking that races the cut and is accepted (decision 4);
- the pure `TxList` tests in `crates/ezsdr-radio/tests/staging.rs`;
- the deletion/rewrite table: every test and mutation row removed, rewritten or retargeted (sketch §3), each with its replacement or the argument; grep every row whose `new` names a removed item and every row whose `filter` names a removed test, and list the commands;
- new mutation rows (Rust token anchors; each names its killer test in `filter`): at least the `Streams::end` halt push, the step's unread guard, the receipt's cut re-check, the list's FIFO order (e.g. `pop_back` for `pop_front`), the Stop's halt push at receipt, and the Mock's halt-before-cut order;
- which O1 tests cover which sequence already (they must stay green unchanged, except the #85 comment).

Do not write code before the orchestrator says the plan is accepted.

## Rules (binding)

- Do not commit, push or `git add`. Do not run `mutate.py` or apply a mutant to the tree; mutation rows run only through `remote_mutate.py`, which the orchestrator runs.
- Builds and tests: `python3 plan/maintenance/tools/remote/rdev.py -- cargo …` in the foreground (long timeouts). Test temp files under /tmp. `PYTHONDONTWRITEBYTECODE=1` for Python.
- Before reporting code: `cargo fmt --all`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 4151197` = 0 (every mutation row's anchor found exactly once; rows anchored in `book_burst` that move into the critical section keep their tokens).
- Mind the guard pitfall (sketch §2): bind `list.read()`'s result with `let` before the body locks again.
- Do not edit `plan/phase8/49-*.md`, `handoff.md`, `AGENTS.md`, `KNOWN_LIMITATIONS.md`, or `design/` beyond what this brief allows. Do not edit the O1 oracle (`crates/ezsdr-acceptance/tests/oracle/`).
- No tests that depend on wall-clock timing. No new FakeDevice faults. Do not delete a test whose sequence is still reachable without a replacement.
- Kill processes only by PID.

## Report

End your final message with: the plan's path (or, after implementation, the files changed), the sequences covered and any not covered with the reason, every spec/code disagreement found (first), the tests and mutation rows added/removed/retargeted, the `design/` sentences that became false with proposed text, and the commands run with pass/fail counts.
