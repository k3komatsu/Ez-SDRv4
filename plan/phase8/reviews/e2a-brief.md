# Brief — note 49 stage E2a: operations and results as one protocol, owned by one transmit book (#80, #84, #85, #86, #88)

Repository: /home/komatsu/local_work/Ez-SDRv4, branch `work/e2a` from `main` (the orchestrator names the commit when launching). Read first: AGENTS.md "Safety core", §3, §6, §7, §9 (you are the implementer; note §9's "Hardware tests and bench values" and the full mutation lists at each sub-step's commit, which the orchestrator runs); `plan/phase8/49-time-and-booking-inventory.md` §5.4, §7, §11, §12, §12.1 (read only; the orchestrator's file); the E1 records `plan/phase8/reviews/e1-sketch.md`, `e1-dispositions.md`; issues #80, #84, #85, #86, #88 (`gh issue view N`).

**The design** (AGENTS §9 step 0, done): `/home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/sketch.md` (an Opus draft; its first section **"Owner decisions"** overrides the rest where they differ) and the design review `/home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/fable-design.md` (Fable 5.2; findings F1–F9 and alternatives A1–A5 are all adopted). Read both in full. Where the sketch and an adopted finding disagree, the finding wins; where you find the sketch or the review wrong against the code, say so with evidence in the plan — do not follow either blindly.

**Risk: risky** (ownership of results, a lifecycle open → issued → ended, the order of device calls against lock sections, a new receipt refusal). So: **first a test plan, no code**; Sol reviews it; then you implement.

## What E2a does (summary; the sketch §1 is the shape)

- One `TxBook<P>` per transmit stream in `ezsdr_radio::staging` owns the bookings (`Bookings`), the receipt list (`TxList`), the repeat switch, `playing`, the open booking and the one in-flight send (`inflight: Option<Send>`, A3: no op ids). uhd-tx keeps device facts (`TxChain`, report correlation, `BurstTracker`, streamer). MockRadio uses the same book.
- uhd-tx plans each send under the streams lock, makes the device call(s) outside it, and hands the outcome back; the book alone turns a short or failed outcome or a broken report into an abandon, which reaches only its own booking and the switch replacing it (#80). A halt cancels an open booking with nothing issued (#85). One switch-cancellation transition removes the switch's payload from `held` and `unread` (A1, F1).
- A switch is decided at receipt: taken at its boundary, or refused at receipt when its repeat's first sample has not been issued (N2 = (b1), owner); nothing moves it later. The UR-38 move, `moved_to`, `Switch.installed`, `Bookings::refuse`, `SwitchOutcome::Refused` go. A timed switch the receipt pushed past the boundary `at` alone gives records `requested_target = at`; an untimed one records none (#86, owner, F2). `playing` follows a repeat moved at dispatch (#88).
- Sub-steps (A2), each green, each committed by the orchestrator on `work/e2a`:
  - **E2a-1** one owner: the book holds `record`, `list`, `playing`; today's write-backs become book transitions with today's semantics. No behaviour change.
  - **E2a-2a** `open` into the book and the send split (plan under the lock / device calls / `complete`), `stats.tx_lock_wait_max_ns`. No behaviour change and **no FakeDevice `calls()` trace change**.
  - **E2a-2b** the abandon scoped to its booking (#80, as a pure function, A4), the halt takes an unsent `open` (#85), `broken` with no open device burst changes nothing (A5, F4).
  - **E2a-3** switches at receipt (`receive_switch` shared by both Providers), (b1), the guard on any unread entry, `dispatched` moves `playing.origin` (#88), the deletions, #86's rule, the spec text, the oracle's #84/#86 answers.

## Operation sequences and expected results

Use the sketch §6's S1–S17 **and** the review's §4 items 1–9 (cancelled switch with an unread payload; the scoped abandon at c = b and the region [b, b + STOP_FRONT); a late underflow with no open device burst; the late-policy order for an unsent head (F8); loss in flight; a refused switch changes nothing; #86 timed-pushed / timed-on-rule / untimed, both Providers; the Mock's MR-24 and tracker error exits change no record (F6); a switch on a repeat behind a finite booking that has not started). Each with its sub-step, the expected result, and what it relies on (VERIFIED/INFERRED). Use the step-F constants (`STOP_FRONT`, `CLOSE_GAP`, `closed_short_gap`), never literal numbers.

Hardware facts (cite, do not re-derive): bench-results.md "2026-11-05", "2026-11-06", "2026-11-10" (gap-0 drop, the safe start after a close, P6 late continuations not sent); reports carry no booking id (UHD `async_metadata_t`, note 49 §8). INFERRED: W 10 ms, device lead 2 ms, allowance 3 ms (KL-15).

## User-facing promises (AGENTS §9 step 1)

The new refusal (a switch on a repeat that has not started on the device) is checked against EA-11, EA-16 (`design/16-easy-api.md`, the `switch` row) and the Python client (`python/ezsdr/session.py` `switch`, its docstring); it gets its own KNOWN_LIMITATIONS entry (owner). List every user-visible change (refusal, removed `moved_to` column, `requested_target` of a pushed timed switch, the fixed issues) with the text that changes.

## Text to change, in the sub-step that changes the behaviour

RM-15/RM-16/RM-21 only if a sentence becomes false; RM-37 (playing, the move sentence, (b1), the untimed sentence, the unreachable clauses the review F5 lists); UR-21, UR-23, UR-30's row, UR-38 (rewrite); MR-42 (`moved_to`); EA-16's row and the docstring; the new KL entry; note 49 §7's RM-37 line is the orchestrator's. Keep every `*Test:*` line naming the tests that pin it; renamed tests go in a renames file.

## Test plan (deliver this first, then STOP and report)

Write `/home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/test-plan.md`, organised by sub-step:
- for each sequence, the test (existing to change, or new: name, file), what it asserts, how it observes the behaviour (receipt answer, Manifest event, FakeDevice `calls()` trace, record state, stat), and the wrong behaviour that would fail it;
- for E2a-2a, how the `calls()` traces of the existing frozen rigs are compared before and after (the main risk: device framing);
- the migration table (sketch §5 corrected by the review F9): tests that disappear (with the evidence their state is unreachable), tests rewritten, assertions moved to pure book tests, the O1 pins that flip and when;
- the mutation rows to delete, retarget, re-anchor and add (`plan/maintenance/tools/mutations.json`; Rust token anchors; each row names its killing test in `filter`; check each killer runs the mutated path);
- the hardware tests whose expectations depend on the switch rule or the abandon (AGENTS §9: an audit table, and a FakeDevice rehearsal for each);
- the test-build checks the review F7 asks for (same-mutex re-entry panics; `emit_at` under the streams lock panics).

Do not write code before the orchestrator says the plan is accepted.

## Rules (binding)

- Do not commit, push or `git add`. Do not run `mutate.py` or apply a mutant to the tree; mutation rows run only through `remote_mutate.py`, which the orchestrator runs.
- Builds and tests remotely: `python3 plan/maintenance/tools/remote/rdev.py -- <cargo …>`, in the foreground until it ends. Temp files under /tmp.
- Before reporting code: `cargo fmt --all`, then `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base <the base commit> [--renames <file>]` = 0.
- Do not edit `plan/phase8/49-*.md`, `plan/phase8/bench-results.md`, `bench/`, `handoff.md` or `AGENTS.md`.
- Do not delete a test of a reachable sequence without a replacement; grep rows whose `new` names a removed symbol and rows whose `filter` names a removed test.
- No test that depends on wall-clock timing.
