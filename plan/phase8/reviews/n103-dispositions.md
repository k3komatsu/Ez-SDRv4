# #103 — dispositions and counts

The step: the start's admission lock (KC-24a), issue #103. Way of working: a Sonnet subagent wrote the test plan and supervised Go DeepSeek as implementer (owner's trial, 2026-11-18). Reviews: Sol 6.2 on the test plan (v1) and on the code; Opus re-checks of plan v2 and of fix round 1. Final test plan: [n103-test-plan.md](n103-test-plan.md); briefs: [n103-briefs.md](n103-briefs.md); report: [n103-report.md](n103-report.md); gate: `n103-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Sol 6.2, test plan v1 | 5 | 3 | 1 | none counted (test design) |
| Opus re-check, plan v2 | 1 | 4 | 3 | none counted |
| Sol 6.2, code | 3 | 4 | 1 | none counted: F1 a refusal-order regression introduced and caught in the step; F2–F3 test observation and harness ordering |
| Opus re-check, fix round 1 | 0 | 1 | 3 | none |

The issue itself is `mech:planned-state-read-as-actual` (three-strikes check below). Follow-ups filed: #110 (an admitted batch dropped by an end before its dispatch is recorded nowhere), #111 (cleanup's freeze calls `cancel()` under the admission lock), #109 (the t200 self-check's wall-clock budget, seen in this step's workspace runs).

## Decisions (orchestrator unless marked)

## The plan's open questions (test-plan.md §9), provisional until Sol's plan review
1. A `feature = "testing"` seam in `src/coordinator/` (inline gated module, two hook points, `admission_held()`): accepted provisionally, decided by the orchestrator — the old window cannot be forced deterministically otherwise; it is test-only and off in every production build; OV-20 forbids doubles in `src/`, and a hook is not a double, but it is new precedent, so Sol is asked to look for a seam-free deterministic alternative. Not a process-rule change.
2. The `frozen` check inside the start's dispatch, unreachable after the change: delete (a leftover).
3. Optional T3a and row N103-6 (the end check right after `move_to(Running)`): include; cheap and in the changed region.
4. The admitted batch silently dropped when an end is set before the dispatch (`:326`/`:330` checks; neither `undelivered` nor a log): pre-existing, outside #103; file as its own issue, not fixed here (unless Sol shows it is the same mechanism).
5. An end in the window now records the batch's ids in `termination.undelivered` deterministically: one of the outcomes the race already allowed, no rule change; no owner question.
6. No new `mech:` label.
7. KC-24a (and KC-18) get the one-clause amendment that states the lock's extent for the start (admission precedes `start` by KC-17; the lock is held from before `Running` through the dispatch): wording that matches the code, no behaviour change; decided by the orchestrator.

## Sol 6.2's plan review (sol-plan.md, 19:42–19:50): dispositions (orchestrator)
- F1 major (deadlock: `move_to(Running)` calls `TimeAuthority::now()` and `HostClock::utc_nanos()` under the admission lock; a Provider worker holding a mutex the clock read needs and submitting closes a cycle): **adopted**. Sample the transition's clocks before taking the lock, then take it and publish `Running` with those values; keep the timestamps and the clock-failure path as today. This revises Q7's wording too (the lock is held from the publication of `Running`).
- F2 major (T6's `wait_for` is a whole-line match; its kill rests on KC-21a's budget): **adopted**. Prefix predicate or a result channel; a non-blocking "admission is free" assertion at the real wait boundary; the timed receive is only a backstop.
- F3 major (a Module worker that submits while `Armed`, blocks on the lock, then passes step 2 after `Running`): **adopted, decided by the orchestrator: the state at admission (KC-24 step 2) governs**, as it already does today for a submission racing the transition; RS-18's "arrives" is read as "is admitted", and RS-18 gets that clause in the wording change. Pin it with a test (worker submits from `start`'s thread after `start` returns, gated so it reaches the lock while `Armed`; it is admitted after the batch with the next ActionId). Recorded as a decision; one line at landing.
- F4 major (T3 can pass with no end in the window; a deaf Provider cannot show a push after the freeze): **adopted**. Observe the end request positively before the start is released; an ordering observation if the test claims the freeze ordering.
- F5 major (no test catches an unlock between scheduled entries): **adopted**. Two scheduled entries with distinct values; the competitor gated at the last dispatch boundary; it sees the last value; dispatch order and the final value asserted; a batch whose admission fails or whose instance's `start` fails publishes none of its updates.
- F6 minor (T2's prefix match; `progressed`/park claim too wide; `Seen` also runs for the schedule's admission outside the lock): **adopted**.
- F7 minor (N103-5's second acquisition self-deadlocks; N103-3/-4 identical edits; rows relying on watchdog expiry): **adopted**. Move the existing acquisition; distinct mutants; non-blocking guard checks before the call that would deadlock; each row's filter is its named killer only.
- F8 minor (the seam): **adopted**: a private `cfg(test)` coordinator probe with `--lib` tests if the doubles the tests need can be reached from a unit test (precedent `time/authority.rs:334`); otherwise the exact-gated inline `testing` module with nothing `pub` beyond what the integration tests call, and the reason stated in the plan.
- F9 note: keep the early-end check, drop the redundant `frozen` read, include T3a/N103-6 with a positive dispatch observation (Q2, Q3 stand). The `undelivered` outcome (Q5) stays the orchestrator's: the Manifest's fields and meaning do not change, one outcome the race allowed becomes the only one; it is listed in one line at landing so the owner can reverse it. KC-24a amended for the pre-start admission, KC-18 cross-references it, KC-46's thread start before the dispatch kept. Mechanism: weighed against `failure-mechanisms.md`'s existing entries at the record (Sol suggests the stale-snapshot one); not decided in the plan.

## test-plan-v2 open questions (orchestrator)
1. The exact-gated inline `start_probe` seam (`Point`, `Probe`, `Reading`, `install` public under `feature = "testing"`): accepted — a private `cfg(test)` seam cannot reach the doubles in `tests/support` (v2's evidence); the probe reads the lock itself.
2. `Authority::cancel()` under `admission` in the freeze (F1's shape, pre-existing): its own issue, with the silent batch drop (Q4) — filed at the gate, not fixed here.
3. Mechanism label: decided at the record.
4. T8's two assertions without a row: kept as drift guards.

## Opus re-check of v2 (recheck-v2.md): FAIL on one major, narrow — dispositions (orchestrator)
- M1 (a `try_lock` read at `Released` means "held by any thread": a parked competitor can take the lock right after the drop; breaks T6's read, `assert_extent`'s `Released` clause in T1/T2a/T2b and `dispatch_frozen == false` in T3 on correct code): **adopted** as the re-check states it — "not held" at `Released` only in tests with no competitor at the drop (T7, T4, T5); `dispatch_frozen` checked only where the lock is held; T6's Executor gated, its `Released` hook reads first, then opens the gate, then asserts. Settled in phase B by looping T6, T2a, T2b, T3 20 times on correct code (their verdict depends on threads).
- m1 (`start:panicked` probe line that ends every wait, so N103-3 dies on its named assertion, not the 10 s expiry; drop "assert the Run is still Running"), m2 (N103-6 releases the lock before `Dispatching` and retakes it after), m3 (T8 asserts `test.grid == 20.0`), m4 (`BeforePublish` in `pipeline.rs` between the lock and `publish`): **adopted**.
- Notes adopted: §4's list of panicking hook points follows T5's record-only wording; T3's Executor does not submit and its hook reads `held` before opening the loss gate; `move_to_at` is `pub(crate)`; N103-3 run once in phase B to check that no double panic aborts the binary.
- No further plan round: v2 + these dispositions are the binding plan; the implementer writes the deltas into `test-plan-v3.md` before the code, and Sol's code review sees the whole.

## Three-strikes check (AGENTS §6), written before the code lands (orchestrator, 2026-11-18)
#103 is `mech:planned-state-read-as-actual` (labelled with its reason on the issue): the start batch's admission verdict, taken against a working copy of the configuration before `start` (KC-17), is reused at dispatch as if still true. The mechanism is far past three; its notes (plan/maintenance/23-one-line-per-stream.md, plan/phase8/reviews/1b4d1c-opus.md §3) answer it for the UHD Module, not for the Kernel's admission. The question for this instance:
- **Can the concept be removed?** Admitting each scheduled entry at its dispatch instead (no verdict held ahead) would move a refusal after `start` — `Failed { arm }` before any instance starts would become a refusal after instances run and may transmit, a user-visible weakening of KC-17. Rejected.
- **Why the held verdict cannot go stale after the fix:** between the batch's admission and its dispatch, nothing can change the configuration — before `Running` every Session and Module submission is refused at KC-24 step 2 (RS-18, the state at admission), and from the publication of `Running` the admission lock is held until the last dispatch (KC-24a). So the working copy *is* the configuration the batch is dispatched onto. The design stands; the tests that pin the invariant: T1 (a submission parked at the transition is admitted after the batch), T2a/T2b (the first data-thread submission, and between two scheduled entries, sees the batch's last value), T8 (a batch that fails publishes nothing).
- **Is the rule implemented twice?** No: one admission function (KC-24) and one lock; the start was the one call whose lock extent did not cover its admission-to-dispatch window. Phase B's survey (plan §6/§7) confirms no other control-path call has the shape.

## Sol 6.2's code review (sol-code.md, 21:18–21:25): FAIL, 3 majors, 4 minors, 1 note — dispositions (orchestrator; all adopted)
- F1 major (`RunStateMachine::move_to`, a public fn, now reads `utc_nanos()` before refusing an illegal transition): adopted — `move_to` validates before it reads the clock, as before; one shared validation/recording path with `move_to_at`; a test with a counting (or panicking) clock on an illegal transition. The coordinator's `publish` keeps sampling first (KC-24a), and its `debug_assert` stays.
- F2 major (T4 observes only `utc_nanos`; N103-7 moves both reads together, so `TimeAuthority::now()` moved back under the lock alone stays green): adopted — observe each callback independently at the `Running` transition (the lock free in both), their order pinned by distinguishable samples, and a row per callback.
- F3 major (N103-3: the worker can mark `start:panicked` before sending its payload, so the test fails on its outer wait, an unmarked harness failure the runner credits as a kill): adopted — the payload is sent before the panic is announced; a real watchdog expiry prints the `EZSDR-INFRA:` marker; fixture gates are released on failure so a parked Provider can exit.
- F4 minor (T1/T2/T6 claim more than they observe; T6 passes if the wait is skipped): adopted — a positive observation at the real submit and wait boundaries, or the claims narrowed to what is observed; T6 asserts that start returns a live Run.
- F5 minor (`Installed` is `Send`; nested installs clobber): adopted — the guard is thread-bound (`!Send`), nesting refused or restored explicitly; `read`'s doc says it briefly locks `end`.
- F6 minor (KC-24a/RS-18 overstate: a submission waiting while the freeze wins is refused at step 1; a Module `Abort` bypasses admission; only Session Actions are logged): adopted — texts qualified; RS-18's Test line cites T5.
- F7 minor (the new start-failure test can pass before `start` is called): adopted — fold the scheduled case into the existing KC-18 start-failure test and assert the Provider's recorded `start` or its failure reason; delete the duplicate fixture.
- F8 note: adopted — `next` from `dispatched.len()`; the unused glob re-export removed; N103-6 narrowed to the gap before entry 1, using the anchor's second replacement for the guard binding.
Mechanism: none of these is a new strike of a counted mechanism (test-observation gaps and a refusal-order regression introduced and caught in the step).
