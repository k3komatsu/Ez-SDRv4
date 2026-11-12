# Note 51 stage 1d — review record and dispositions (2026-11-12)

The UHD lib tests that hold a `Frozen` participant and then release it hand the clock over to the test thread first: a test-only helper `test_support::handed_over(frozen, &clock) -> Driver` (crates/ezsdr-radio-uhd/src/provider/test_support.rs) registers the calling thread on the manual clock and then releases the frozen participant, so the clock never runs between the two. The four G sites in src/provider/tx.rs (the test thread runs owner code after the release; one of them, `drop(rig._frozen)` in `ur_22_f8_…`, was missing from note 51's table) use it; `ur_28_…`, the one site whose verdict depended on when the test thread ran (a 3 ms stall failed it), sleeps 50 ms after the hand-over and asserts the clock did not move. The three W2 sites (tx.rs, tx/tests/e1.rs `shutdown`, tx/tests/e2b.rs `shutdown_owner`, the last added by E2b) lose the 150 ms sleeping helper thread and its `join`. N, W1 (a guard would hang, now VERIFIED) and H stay. Note 51 §4.4 now counts 17 sites (4 N, 4 G, 5 W1, 3 W2, 1 H). Risk: risky (small; a lifecycle: the test thread's participant registration). Branch `work/n51-1d` from `bfd59d3`, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n51-1d-briefs.md](n51-1d-briefs.md)); test plan [n51-1d-test-plan.md](n51-1d-test-plan.md).

## Process

- Test plan with out-of-tree probes (delay injection, diagnostic only; the W2 rewrite tried in a copy); Sol's plan review ([n51-1d-sol-plan.md](n51-1d-sol-plan.md)): PASS WITH CHANGES, three minors, all adopted.
- Implementation in one run (1d-A helper, 1d-B G sites, 1d-C W2 sites); the 80 affected tests 20 runs; rows N51D-01…03 and the re-run rows killed by assertions.
- Full mutation lists before Sol's code review: 2286/2287 (only `phase7/G09`).
- Sol's code review ([n51-1d-sol-code.md](n51-1d-sol-code.md)): PASS WITH CHANGES, one minor (pre-existing).

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan | 0 | 3 | `wall-clock-test-oracle` (m1: an unmarked setup bound) — plan defects |
| Sol, code | 0 | 1 | `wall-clock-test-oracle` (a row killed only by a test's real-time bound, counted as an ordinary kill; pre-existing since E2a-2a) |

## Dispositions

**Sol, test plan (PASS WITH CHANGES)** — all adopted ([n51-1d-briefs.md](n51-1d-briefs.md) "Orchestrator decisions on the plan"): m1 the helper test's 10 s setup bound writes the infra marker on `Timeout` only; m2 `ur_28_…` asserts the clock did not move right after the slow step, so N51D-03 dies on that assertion; m3 negative diagnostic claims read "no dependence observed at these injection points and durations". 1d-C′ (three sleeping-helper tests with no `Frozen`) is deferred to stage 4, as Sol advised.

**Sol, code (PASS WITH CHANGES)**
- Minor, row E2A2A-10 (a thread re-entering a Core mutex hangs instead of panicking) was killed only when `e2a_lock_reentry_of_one_mutex_panics`'s 10 s `recv_timeout` expired, with no marker, so the runner counted an ordinary kill: fixed by the orchestrator, the timeout writes the `EZSDR-INFRA:` marker (a disconnected worker stays a plain failure) and the row declares `"kill": "bound"` (a deadlock is seen only by the bound); re-run: `killed (bound)`.

## Decided by the orchestrator

- One implementation run and one commit for 1d-A, 1d-B and 1d-C (about 90 lines): the per-sub-step review was waived for this small step, and the self-review too (Sonnet implementer, owner 2026-11-03); Sol's code review remained.
- The guard is the `drive()` hand-over helper the tables name, not #76's `released` shape; note 51 §4.4 says so.
- 1d-C′ deferred to stage 4; its four rows (P1298, P1302, P1306, P1312) untouched.

## Gate

PASS ([n51-1d-gate.json](n51-1d-gate.json)) on sim02:8 and sim03:8: 2454 tests, 600 UHD, Python 121 × 2, e2e 9/9, mutation 2286/2287 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, E2A2A-10, N51A-14, N51A-15, N51B-08, N51C-06, `phase7/G16`, no `ERROR`, no timing re-runs.
