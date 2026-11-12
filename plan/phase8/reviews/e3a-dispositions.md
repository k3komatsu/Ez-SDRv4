# Note 49 E3a — review record and dispositions (2026-11-12)

The transmit segmentation moves into pure functions, `crates/ezsdr-radio/src/segment.rs` (`send_cap`, `extent`, `plan_send`, `handed_end`) with `x3xx::PACKETS` (`PacketRules { min_open_first_send: 32, min_first_send: 4 }`); the UHD Provider's send and step and MockRadio's blocks, wake and Stop model call them. No behaviour change. E3 was split (decided by the orchestrator, [e3a-briefs.md](e3a-briefs.md) "Orchestrator decisions", K1): E3a is this extraction; E3b moves `TxChain` into the book and gives the record's front the same arithmetic, with its own plan. Risk: risky (the boundary between the fake and the hardware it stands for; every transmit row re-anchored). Branch `work/e3` from `bdf3493`, rebased onto `965dd63`, five sub-steps (E3a-1 characterization tests on the old code, E3a-2 the pure functions, E3a-3 UHD wiring, E3a-4 Mock wiring, E3a-5 removal of the transitional copies), one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([e3a-briefs.md](e3a-briefs.md)); test plan [e3a-test-plan.md](e3a-test-plan.md).

## Process

- Test plan with an out-of-tree probe; Sol's plan review ([e3a-sol-plan.md](e3a-sol-plan.md)): FAIL, two majors, three minors, all adopted; Opus re-check of the amended plan: two majors, three minors, all adopted (the plan's "Changes after the Opus re-check" section).
- Characterization tests (C1–C6, U1, MC1–MC5, table Q, rehearsals R1–R4) green on `bdf3493` before any production edit; the transitional verbatim copies of the old arithmetic (P-G) compared old and new through both wiring sub-steps and were deleted in E3a-5.
- Out-of-tree trace comparison, base against branch, four runs each (UHD lib, Mock and acceptance): no named file differs; see the open point below.
- Full mutation lists before Sol's code review: 2192/2193 (only `phase7/G09`). Sol's code review ([e3a-sol-code.md](e3a-sol-code.md)): PASS WITH CHANGES, one minor.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan | 2 | 3 | `wall-clock-test-oracle` (M1: a paired comparison relied on margins and repetitions); M2 a test that did not observe the wake it claimed — plan defects |
| Opus, plan re-check | 2 | 3 | — (plan defects: coordinates of table Q, MC5's deadline) |
| Sol, code | 0 | 1 | — |

## Dispositions

**Sol, test plan (FAIL)** — all adopted (decided by the orchestrator, [e3a-briefs.md](e3a-briefs.md) "After Sol's plan review"): M1 no margins or repetitions as the timing guarantee (frozen or stepped rigs, Stop at a stated device instant); M2 MC5 observes the scheduled wake deadline and the block; m3 MC4 ordered `(first_sample_time, len, flags)` snapshots; m4 equivalent mutants replaced, an old-row → replacement table; m5 `hw_p70` and `hw_p72` rehearsals (R3, R4).

**Opus, plan re-check** — all adopted; closed as the test plan's table says (table Q in the Stop's coordinates; MC5's deadline from `v_after`; MC4 rows only when `tx_blocks` grows; the redundant `start < we` guard dropped from `handed_end` so the mutant is observable; R1–R4 on the manual-clock fake).

**Sol, code (PASS WITH CHANGES)**
- Minor, `Plan::short_first`'s doc said such a send closes the device burst; it closes only when the Kernel burst ends with it (finite 33 with a window of 32: `short_first`, no `device_eob`): fixed by the orchestrator, the doc names the size class and points to `device_eob`.
- Open point (the implementer's trace: a MockRadio dropped on an unnamed thread in acceptance binary `v58` in 2 of 4 branch runs, 0 of 4 base runs): Sol judged it noise, not this branch — cleanup workers hold `Arc<Shared>` and are not joined (`crates/ezsdr-kernel/src/run.rs`, `coordinator/ending.rs`), so the last owner can drop on either thread; the affected cases are validation refusals that never segment; contents identical in all eight runs. Accepted.

## Decided by the orchestrator

- E3 split into E3a and E3b (K1); the packet values in one `PacketRules`, FakeDevice keeps its own literals (K2, K3, K7); the Mock's Stop model is the window-only mode (K4); duplicate rows removed (K8).
- Note 49: §5.4.8's Mock parenthetical rewritten to the behaviour E3a keeps (K5: an open booking's first send of one sample, the held start not read, a joined piece a fresh first send); its `tx.rs`/`lib.rs` line cites are left as of `bdf3493`, with a Changes row pointing to the "Changes in E3a-5" table of [e3a-test-plan.md](e3a-test-plan.md) for where the code is now.
- Six `file.rs:N` cites in tests that concern `control.rs` and the fake, not E3a's code, left unchanged.

## Gate

PASS ([e3a-gate.json](e3a-gate.json)) on sim02:8 and sim03:8: 2394 tests, 590 UHD, Python 121 × 2, e2e 9/9, mutation 2212/2213 (the known survivor `phase7/G09`), no `ERROR`, no timing re-runs.
