# Note 51 stage 1c — review record and dispositions (2026-11-12)

The `Direct` test rig (crates/ezsdr-radio-uhd/tests/fake.rs) registers its driving test thread on its manual clock (stage 1a's `HostClock::drive()`), as `UhdRig` did in 1b, so every `Direct` test's verdict no longer depends on when the test thread runs; the README's `Direct` exception and note 37 decision 4's wall-clock allowance are gone. One wait rule for both rigs replaces 1b's per-method 1 µs: **R1**, a wait that ends on an instant the test chose (`settle`, `wait_until`, `Direct::start`) ends 1 µs past it, so every participant due at that instant runs first; **R2**, the rig's Action queue (`Stamped<T>`, crates/ezsdr-radio-uhd/tests/common/rig.rs) hands an Action out only at a clock instant strictly later than its push, so a receipt is the first poll after the push. Waits that end on an observed state (`Direct::until`, `UhdRig::wait_for_events`) take no R1; both have a device-time horizon that fails with no infra marker and a real-time bound that writes `EZSDR-INFRA:`. W13 (`ur_26_a_receive_start_queued…`) holds its owner on the device clock with an observable entry/release handshake. Both rigs share one drop-order guard (`AuthorityGuard`, `assert_joined`) in tests/common/rig.rs. Risk: risky (a lifecycle: the rig's participant registration and drop order). Branch `work/n51-1c` from `6301ddc`, three sub-steps (1c-A waits, bounds, R2 and W13; 1c-B the `Direct` driver; 1c-C the shared guard, a pure move), one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n51-1c-briefs.md](n51-1c-briefs.md)); test plan [n51-1c-test-plan.md](n51-1c-test-plan.md).

## Process

- Test plan with out-of-tree probes; Sol's plan review ([n51-1c-sol-plan.md](n51-1c-sol-plan.md)): FAIL, three majors, all adopted; the amendments and sub-step A were one run.
- Each sub-step: tests local, its rows on sim03, a local commit. T4 and the `direct_rig_` group 96 runs under load (six processes); the `Direct` tests 21 runs; the `uhd_rig_` group 20 runs; 0 failures.
- Full mutation lists before Sol's code review: 2239/2240 (only `phase7/G09`).
- Sol's code review ([n51-1c-sol-code.md](n51-1c-sol-code.md)): PASS WITH CHANGES, one minor.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan | 3 | 0 | `wall-clock-test-oracle` (M1: W13 could pass without a Stop during configuration; M3: a fixed epsilon leaves a tie when the wait's end lands on a poll) — plan defects; M2 a probe artifact (a stale binary), not a defect |
| Sol, code | 0 | 1 | — (text) |

As in 1b, these are findings in the step's own tests and plan, under the mechanism note 51 is the design note for; no production code changed.

## Dispositions

**Sol, test plan (FAIL)** — all adopted ([n51-1c-briefs.md](n51-1c-briefs.md) "After Sol's plan review").
- M1, W13's rewrite let the owner's 10 s hold release the configuration before the Stop: the owner waits for its release on the device clock; the test asserts the owner is held before and after `Provider::stop`; an expired hold writes the infra marker. P473 is killed by "the owner still issues its timed start".
- M2, the "unexplained" T4 race (14/96 in one sketch build) came from a probe binary built with M12's bypass (`Direct::start` calling the raw `wait_until`): not a stage-1a clock defect. The `start()` duration assertion and M12's coverage are mandatory (N51C-12); probe copies are rebuilt from source every time.
- M3, a per-method epsilon is not general (`settle(999 µs)` from an armed rig ends on a poll instant): R1 plus R2 above, stated once in the README; the `settle(999 µs)` boundary case on both rigs. The boundary tests are regression pins; the deterministic killers of R2 are the `Stamped` unit tests and `…_is_not_received_at_the_instant_it_was_pushed` (N51C-14, N51C-18).

**Sol, code (PASS WITH CHANGES)**
- Minor, note 51's contract 2 said a receipt is the first poll after the requested T, while R2 gives the first poll after the push instant (T + 1 µs; one poll later when T + 1 µs is a poll instant): fixed by the orchestrator, contract 2 names the push instant with Sol's example.
- Notes accepted as Sol stated them: N51B-09 and N51C-10 share one `old` text in rig.rs and are two rows because each runs a different test binary; N51C-06 is the only bound kill in the step's rows besides 1b's N51B-08, and both declare it.

## Decided by the orchestrator

- Sub-steps A and B as planned, C adopted (one guard for both rigs); D (the real-clock `Gated` waits) deferred to stage 3; the horizon on `UhdRig::wait_for_events` added in A ([n51-1c-briefs.md](n51-1c-briefs.md) "Orchestrator decisions on the plan").
- R1 does not apply to waits that end on an observed state (`Direct::until`, `UhdRig::wait_for_events`): their end is a latency, not a verdict (1c-A's contradiction 2).
- Three 1b assertions now expect 7 ms + 1 µs for `settle(7 ms)`: the one rule replaces 1b's exact-duration claim (1c-A's contradiction 1); 1b's records stay as history.
- No Opus re-check: no major in the code review, and Sol's code review checked the plan majors' fixes on the code.

## Gate

PASS ([n51-1c-gate.json](n51-1c-gate.json)) on sim02:8 and sim03:8: 2452 tests, 598 UHD, Python 121 × 2, e2e 9/9, mutation 2283/2284 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, N51A-14, N51A-15, N51B-08, N51C-06, `phase7/G16`, no `ERROR`, no timing re-runs.
