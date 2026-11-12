# Note 51 stage 1a — review record and dispositions (2026-11-12)

Per-clock participant registration on the manual clock (`crates/ezsdr-radio-uhd/src/host.rs`): a thread holds `(clock, id)` pairs; `HostClock::drive()` returns a `!Send` `Driver` that registers the calling thread and unregisters on drop (also on unwind); a second registration of the same thread on the same manual clock panics before allocating (Q7); a real clock gives a no-op guard. `UhdRadio::join`'s 1 ms wait is a clock wait, `JOIN_BOUND` stays real time (Q2). No rig uses `drive()` yet (1b, 1c). Risk: risky (participant identity, a lifecycle). Branch `work/n51-1a` from `1231462`, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n51-1a-briefs.md](n51-1a-briefs.md)).

## Process

- Test plan ([n51-1a-test-plan.md](n51-1a-test-plan.md)) with an out-of-tree probe of 15, then 19 mutants. Sol's plan review ([n51-1a-sol-plan.md](n51-1a-sol-plan.md)): FAIL, two majors; adopted ([n51-1a-plan-adjudication.md](n51-1a-plan-adjudication.md)), the plan amended.
- Opus re-check of the amended plan: M1, M2 closed; minors (test 4 must assert `ids_on`, so row 08 dies on an assertion rather than a 5 s bound; a vacuous time check in test 7; stale plan text) sent to the running implementer.
- Implementation: 19 tests, each 20 runs; 22 rows (N51A-01…17, J278, J283, J284, J301, J313) killed. Full mutation lists: 2188/2189 (only `phase7/G09`).
- Sol's code review ([n51-1a-sol-code.md](n51-1a-sol-code.md)): FAIL, one major; fixed; Opus re-check: closed, one minor (fixed by the orchestrator).

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan | 2 | 2 | — (plan defects) |
| Opus, plan re-check | 0 | 4 | — |
| Sol, code | 1 | 0 | `wall-clock-test-oracle` (a row killed only by a test's wall-clock bound, counted as an ordinary kill) |
| Opus, code re-check | 0 | 1 | — |

## Dispositions

**Sol, test plan (FAIL)** — all adopted.
- M1, no test waited on the earlier clock with both registrations live (a last-entry-only lookup passed): E9a–E9c (sleep, clock wait, lock queue; both `Driver`s live, colliding ids, target first and second) and row N51A-17.
- M2, setup races in J1, J2, E2 and test 7: the driver registers before any sleeper; J1 handshakes the worker's pending sleep before `join`; J2 observes the wait through the `pause_next_wake` hook and takes its elapsed time inside the joiner; E2's wall sleep removed.
- Minors: probe claims restated from the logs; UR-16's wording "the joining participant is idle during the clock wait; the join bound remains real time".

**Sol, code (FAIL)**
- Major, rows N51A-02, 03, 04, 11, 14, 15 were killed only when a lib-test hang bound expired, with no infra marker, so the runner counted ordinary kills: 02, 03, 04 and 11 now die on a participant-state assertion; 14 and 15 (genuinely bound-only) keep `"kill": "bound"` and their bound writes an `EZSDR-INFRA` marker (`provider/test_support.rs` `infra`, used by `bounded` and J2) — `killed (bound)`.

**Opus, code re-check (closed)**
- Minor: J2 treated a joiner that panicked before any wake (`Disconnected`) as a bound: fixed, only `Timeout` is a bound.
- Note: the lib-test bounds in `host.rs` (`within`, `bounded`, `joined`, `until_blocked`) print no marker; no row relies on them now (not verified row by row). Recorded for note 51 stage 3.

## Decided by the orchestrator

- The real-clock `drive()` is a no-op (a nested second `drive()` on a real clock is not detected; the duplicate-registration wording is scoped to manual clocks).
- Plan-level minors went to the running implementer instead of another plan round.

## Gate

PASS ([n51-1a-gate.json](n51-1a-gate.json)) on sim02:7 and sim03:7: 2365 tests, 582 UHD, Python 121 × 2, e2e 9/9, mutation all rows but the known survivor `phase7/G09`.
