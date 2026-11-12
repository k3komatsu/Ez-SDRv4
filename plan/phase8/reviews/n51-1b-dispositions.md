# Note 51 stage 1b — review record and dispositions (2026-11-12)

`UhdRig` (crates/ezsdr-acceptance/tests/parity.rs) registers its driving test thread on its manual clock with stage 1a's `HostClock::drive()`: the `Driver` is the first field and registers as the constructor's last step, so the device clock advances only while the test thread is idle in the clock. `TxBlocks` is refused at construction; the 10 s `wait_for_events` bound writes the `EZSDR-INFRA:` marker; `wait_until` ends 1 µs past its tick so every participant due at the tick runs first, and an Action pushed after it is received at the next poll. The 14 receipt-window tests #96 left to note 51 pass whatever the test thread's scheduling; their bodies are unchanged. New tests pin the stand-still clock, `settle`/`wait_until`, two rigs on one thread, the receipt instant, and the drop order (N4/N5, with the Authority and a drop probe in one `AuthorityGuard` field). Risk: risky (a lifecycle: the rig's participant registration and drop order). Branch `work/n51-1b` from `2c0493c`, rebased onto `0287fce`, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n51-1b-briefs.md](n51-1b-briefs.md)); test plan [n51-1b-test-plan.md](n51-1b-test-plan.md).

## Process

- Test plan with out-of-tree probes (delay injection, diagnostic only); Sol's plan review ([n51-1b-sol-plan.md](n51-1b-sol-plan.md)): FAIL, two majors, one minor, all adopted; the plan was amended and implemented in one run, and the Opus re-check checked the majors on the code (decided by the orchestrator, to save a round).
- Full mutation lists before Sol's code review: 2197/2198 (only `phase7/G09`).
- Opus re-check of the plan majors on the code ([n51-1b-recheck.md](n51-1b-recheck.md), first part): both closed; it found the receipt tie (T or T+1 poll) and four minors.
- Sol's code review ([n51-1b-sol-code.md](n51-1b-sol-code.md)): FAIL, two majors. Fix round 1 (implementer), then the Opus re-check of the fix (second part): three of four closed, two small changes made by the orchestrator.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan | 2 | 1 | `wall-clock-test-oracle` (M1: N4/N5's state from transient counts; M2: a row killed only after a real-time bound) — plan defects |
| Opus, re-check on the code | 0 | 5 | `wall-clock-test-oracle` (the receipt tie: which of two threads released at one instant runs first) |
| Sol, code | 2 | 0 | `wall-clock-test-oracle` (M1: N4/N5's baseline depended on when the test thread reached `drive()`; M2: N51B-03's kill path ran through `CLOCK_JOIN`) |
| Opus, re-check of fix round 1 | 0 | 2 | `wall-clock-test-oracle` (row N51B-10's kill was a race) |

The mechanism is the one note 51 is the design note for; these are findings in the step's own tests, not new bugs of production code.

## Dispositions

**Sol, test plan (FAIL)** — all adopted ([n51-1b-briefs.md](n51-1b-briefs.md) "After Sol's plan review"): M1 N4/N5 establish uhd-clock asleep inside its read explicitly and observe the join before releasing the evidence; M2 N51B-03 by an ordering assertion; m3 the comment describes the poll phase.

**Opus, re-check on the code** — M1, M2 closed. The receipt tie: a receipt landed at T or one poll later, decided by which thread ran first; `s14_…` S3 had been moved from 71 to 72 ms. Adopted: `wait_until` ends 1 µs past its tick; S3 restored to 71 ms (right by construction, re-check 2). Minors: m1 the baseline before the fault, m2 the drop verdict by an ordering fact, m3 one wrapper field, m4 the README — all fixed in fix round 1.

**Sol, code (FAIL)** — both adopted, fixed in fix round 1.
- M1, a construction stall before `drive()` could let the unreachable fault start a read and inflate the baseline: `FAULT_AT` is 2 s of device time; device time before the fault is asserted at both readings (infra marker otherwise); `idle` is the smaller of two counts 1 clock-ms apart (re-check 2: one of the two is clean, since uhd-clock holds the reference only while busy and reads every 100 ms).
- M2, N51B-03 died only after `CLOCK_JOIN` expired: the probe records the device clock; with the driver last the clock does not advance across the drop, a plain assertion for any `CLOCK_JOIN`.

**Opus, re-check of fix round 1** — fixed by the orchestrator:
- A correct-code detach caused by a host stall failed plainly (the marker branch was unreachable): the marker is written when one reference is left while the clock advanced; N51B-03 (clock still) and N51B-09 (two references) stay plain kills.
- N51B-10 (the 1 µs sleep removed) was killed only when the test thread won a race: `uhd_rig_settle_advances_…` now asserts `wait_until` returns at least 1 µs past its tick, a deterministic killer (20/20 runs pass; the row is killed); the pin test stays as a regression test.
- Residual (accepted): a host stall of 2 s or more during construction still panics plainly.

## Decided by the orchestrator

- The plan amendment and the implementation in one run; the Opus re-check covers the plan majors on the code.
- The receipt rule (1 µs past the tick) in the rig, not a tie rule in `host.rs` (stage 1a's design).
- Note 51's contract 2 and the tables' W3 line now state the 1b facts; a "段 1b の後" paragraph records the corrections to §1.3/§7.1 (the 14 fail at `submit` and `s14`), #96's "15" (14) and §4.2.

## Gate

PASS ([n51-1b-gate.json](n51-1b-gate.json)) on sim02:8 and sim03:8: 2404 tests, 590 UHD, Python 121 × 2, e2e 9/9, mutation 2222/2223 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, N51A-14, N51A-15, N51B-08, `phase7/G16`, no `ERROR`, no timing re-runs.
