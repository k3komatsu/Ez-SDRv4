# E2b — review record and dispositions (2026-11-12)

Note 49 stage E2b: `Configure` and `Reopen` become operations of the transmit book (`TxBook`), issued with a token and completed by naming it; the configuring booking stays in the book; payload issue requires a completed Configure. **No behaviour change.** Risk: risky (a lifecycle). Branch `work/e2b` from `9c84502`, landing on `main` as one squashed commit.

## Process

- Test plan: OpenCode Opus 5.5 #medium ([e2b-briefs.md](e2b-briefs.md), [e2b-test-plan.md](e2b-test-plan.md)). Sol's plan review ([e2b-sol-plan.md](e2b-sol-plan.md)): FAIL, three majors; the plan was amended; the Opus re-check ([e2b-plan-recheck.md](e2b-plan-recheck.md)) closed them and found N1 (major: a plan sequencing error, not a production bug) and N2–N6.
- E2b-1 (Opus #medium): characterization tests C0–C9, R2a, R2b and a test-build seam `on_next_lock`, each green on today's code with a full ordered snapshot (device calls, `applied`, `bursts`, `rejected`, `switches`, `timing`, `device_async`, events with payloads; only the lock-wait stat and the events' source, severity and time excluded). Not reviewed per sub-step (test code only; decided by the orchestrator); Sol's code review covered it.
- E2b-2 (OpenCode Sonnet 5.5 #high, owner 2026-11-12: Opus burned credits too fast): the production change. The E2b-1 snapshots stayed unchanged. Trace comparison (device calls, records and events per test, base `9c84502` vs the branch, 4 runs each): no difference on 3613 stable groups (one real-clock-flaky group excluded with its evidence).
- Full mutation lists before Sol's code review: 2171/2172 (only the known survivor `phase7/G09`).
- Sol's code review ([e2b-sol-code.md](e2b-sol-code.md)): PASS WITH CHANGES, one minor.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan | 3 | 4 | — (plan defects) |
| Opus, plan re-check | 1 | 5 | — (plan defects) |
| Sol, code | 0 | 1 | — |

## Dispositions

**Sol, test plan (FAIL)** — all adopted ([e2b-plan-adjudication.md](e2b-plan-adjudication.md)).
- M1, no operation identity: tokens (`OpId`) on every operation; stale, duplicate, wrong-token and wrong-kind completion panic before any write (P7, rows E2B-15, E2B-17). E2a's decision (d1) "tokens on Sends" had not been implemented; the owner decided tokens on Send, Configure and Reopen in E2b (2026-11-12).
- M2, merging the refused-configure sections would change which receipts are refused in the gap: today's sections and their order are kept; C8 books an overlapping burst in the gap (refused) and after the turn (accepted); row E2B-16 merges the sections.
- M3, E2B-11's killer did not reach the mutation: C9 publishes a loss just before the result section's lock (the seam) for reopen and configure; rows E2B-11, E2B-14.
- Minors: full snapshots; timing through the participant-based `released`; R2a/R2b; the Close rationale corrected; (d1) narrowed; note 49's Configure-refusal wording.

**Opus, plan re-check** — all adopted (decided by the orchestrator).
- N1 (major): `take` and `end_open` assert nothing is in flight; `txbook_take_untake_and_end_open_…` completes S1 before `end_open` (the one existing test changed); P8, E2B-18, and E2B-21 (the `take` assert).
- N2: one `should_panic` function per case with its exact message. N3: C9(c) and E2B-19. N4: C7, C9(a), C9(c), R2b run 20 times. N5: C8 checks the hooks ran and holds no `Control` guard. N6: E2B-20.

**Sol, code (PASS WITH CHANGES)**
- Minor: `e2b_r1_a_first_send_of_a_generation_the_device_does_not_hold_panics_in_the_test_build` expects a `debug_assert!` panic and fails in a release test build: fixed, `#[cfg(debug_assertions)]`.

## Owner decisions (2026-11-12)

- Tokens on all three operations (above).
- A device loss in the middle of a configure (between two channels) drops the booking with no rejection and no note: kept in E2b, filed as #98 (`mech:stream-lifecycle-scattered`, the eighth; a local application of the one book, fixed after E2b).
- A failed reopen cancels an open booking that already sent: kept as today, no issue (production reachability INFERRED; only a synthetic test reaches it).

## Decided by the orchestrator

- Close is not an operation now (no booking-level fact reads its result); revisit at E3 or C6.
- E2b-1 had no sub-step review (test code only).
- Note 49 §5.4.4 and §5.4.6 now separate a loss from a non-lost configure refusal (#98); no `design/` text changes (UR-26, UR-22, UR-36, UR-38, RM-16, MR-33 still true).

## Gate

PASS ([e2b-gate.json](e2b-gate.json)): 2328 tests, 563 UHD, Python 121 × 2, mutation 2171/2172 (`phase7/G09` known), on sim02:7 and sim03:7.
