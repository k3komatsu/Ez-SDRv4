# Note 49 E3b — review record and dispositions (2026-11-12)

The UHD Provider's device-burst state (`TxChain`: the device burst, its next sample, the held tail, the front, recovery) moves into the transmit book as `DeviceChain<T>` in `TxBook<P, T>` (`crates/ezsdr-radio/src/staging.rs`), written inside the sections `complete`, `end_open`, `close_unjoined`, `abandon` and `complete_reopen` already take; the front arithmetic is one set of pure functions in `crates/ezsdr-radio/src/segment.rs` (`close_front`, `predicted_front`, `may_have_handed`). Receipt keeps reading the record's prediction (shape S). No behaviour change; the stat `tx_close_past_record_front` stays; the Mock never drives the chain. Risk: risky (a lifecycle and its ownership: thread-private state becomes lock-shared). Branch `work/e3b` from `0287fce`, rebased onto `6301ddc`, four sub-steps (E3b-1 characterization on the old code, E3b-2 one arithmetic, E3b-3 the move, E3b-4 the fold), one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([e3b-briefs.md](e3b-briefs.md)); test plan [e3b-test-plan.md](e3b-test-plan.md).

## The finding that shaped the step

The test plan found that the record's front is a prediction made at receipt and the executor's is the actual one after a close; they agree for two close shapes only and differ in six sequences (F1–F6). "Receipt reads the executor's state" and "no behaviour change" cannot both hold. **Owner, 2026-11-12: shape S** (「E3bについてSでお願いします」): one arithmetic, receipt keeps the prediction; the shape that reads the state (L) would change user-visible receipt answers and is not part of E3b. F4 — a piece joined to a finite burst abandoned by a short send is accepted as joined and sent alone, so a later booking accepted at receipt is dropped at dispatch — was filed as #99 (mech:planned-state-read-as-actual) by the orchestrator.

## Process

- Test plan with out-of-tree probes; Sol's plan review ([e3b-sol-plan.md](e3b-sol-plan.md)): PASS WITH CHANGES, one major, three minors, all adopted; the plan amended in E3b-1.
- Each sub-step committed on `work/e3b`; rows run per sub-step (42, 42, 61 rows, all killed). Out-of-tree trace comparison, base against E3b-4's tree, four runs each: Mock and acceptance identical; UHD: the chain state line identical in all 237 state files but one wall-time count; seven files differ in single runs (five vary at base too; two tests that build no `Tx` chain differed in 2 of 20 branch runs).
- Full mutation lists before Sol's code review: all killed but `phase7/G09` and two rows whose `new` text still named the old chain (`N47S4-15b`, `P1199`): their `new` fixed by the orchestrator to the same program change through the book, both killed.
- Sol's code review ([e3b-sol-code.md](e3b-sol-code.md)): PASS, no findings.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan | 1 | 3 | `wall-clock-test-oracle` (M1: recovery and shutdown wait on the host clock a frozen rig stops) — a plan defect |
| Sol, code | 0 | 0 | — |
| Test plan (implementer) | — | — | `planned-state-read-as-actual`: F4 filed as #99; F1, F5 recorded (conservative refusals) |

## Dispositions

**Sol, test plan (PASS WITH CHANGES)** — all adopted (decided by the orchestrator): M1 clock-waiting operations run through the `released` participant helper; m1 ordered snapshots at the section boundaries (C3b's seams) with the permitted differences stated; m2 P-MH's implication qualified by "no preceding open join", the counterexample pinned; m3 checked receipt vs saturating executor arithmetic, with boundary rows for each close shape.

**Sol, code (PASS)** — evidence caveats only: the two branch-only trace differences are judged scheduling noise (INFERRED; their tests build no `Tx` chain); the two compile-error rows were repaired and re-run (both killed). `DeviceEnd::Continue` is redundant in reachable owner states but keeps the open-burst assertion: kept.

## Decided by the orchestrator

- K2: the stat stays (removing a Manifest `stats` key is the owner's); its removal goes to the Phase 8-end review with the evidence that it never fires outside its own test.
- K3–K7 as the plan proposed (the Mock does not drive the chain, the tail payload as a type parameter, `Tx::new` resets the chain, F4 not fixed here, the sub-step split); #99 is not listed in KNOWN_LIMITATIONS (an issue tracks it).
- The trace comparison once, at E3b-4, covering E3b-2…4; C4's section counts back at the E3b-1 base after the fold.
- Note 49: §3 R3's "receipt decides with the same function" and §4's "the stat is pinned at 0" corrected; a Changes row.

## Gate

PASS ([e3b-gate.json](e3b-gate.json)) on sim02:8 and sim03:8: 2430 tests, 598 UHD, Python 121 × 2, e2e 9/9, mutation 2266/2267 (the known survivor `phase7/G09`), no `ERROR`, no timing re-runs.
