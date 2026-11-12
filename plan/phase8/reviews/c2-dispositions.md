# Note 49 C2 — review record and dispositions (2026-11-12)

A repeat switch's Manifest outcome now follows whether the sample at its boundary `b` was sent, on both Providers (note 49 §11.10, §5.4.5; RM-37, UR-38, MR-42; KL-42 gone, KL-41 one clause). A pending switch carries what became of its *boundary send* (`Carried`: unissued, in flight, sent, unknown) and, once halted, the halt (`Halted { at, stop, played }`), in the transmit book (`crates/ezsdr-radio/src/staging.rs`). Outcomes:
- `applied` when the boundary send was taken (whole, or short with at least one sample) and device time reaches `b`, or a Stop or `Provider::stop` with bound `B > b` follows it and no loss at a sample at or before `b` comes before device time reaches `b` (the Manifest write applies a switch still waiting so);
- `cancelled` when a Stop with `B ≤ b` (at once, even while the send is in flight), a loss at a sample at or before `b`, an unissued or short-0 send, an abandon at or before `b`, or a failed reopen comes first;
- `uncertain` (new; Manifest only, owner Q2) when the boundary send ended in an exception, or has no result when the Manifest is written (its owner panicked or was left detached); an exception wins over a loss at or before `b`.
A Stop or loss while the boundary send is in flight waits for its result; a loss settles by device time at the loss (owner Q1), also after `Provider::stop`; the owner's abandon halts at device time and keeps its own footprint. The Mock settles at the Stop by its payload cut (no loss can follow a Mock Stop; MR-42). The hardware verdict reads `uncertain` as Inconclusive; `Session.switch()`'s docstring names the three outcomes. Risk: risky (a lifecycle and the boundary between the fake and the hardware). Branch `work/c2` from `fbde789`, rebased onto `42f5031`, sub-steps C2-1 (oracle), C2-2 (the rule), C2-3+4 (hardware verdict), C2-5 (the unreachable abandon rewrite removed), three fix rounds, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([c2-briefs.md](c2-briefs.md)); test plan [c2-test-plan.md](c2-test-plan.md).

## Process

- Test plan with out-of-tree probes; owner decisions Q1, Q2, Q4, Q5 ([c2-briefs.md](c2-briefs.md) "Decisions on the test plan's questions"); Sol's plan review ([c2-sol-plan.md](c2-sol-plan.md)): FAIL, one major, three minors, all adopted in C2-1.
- Each sub-step committed on `work/c2` with its rows; full mutation lists before Sol's code review: 2284/2285 (only `phase7/G09`).
- Sol's code review ([c2-sol-code.md](c2-sol-code.md)): FAIL, two majors, one minor. Fix round 1; Opus re-check ([c2-recheck.md](c2-recheck.md) part 1): both closed, one new finding (a Stop's `applied` final before device time reached `b`). Fix round 2; re-check part 2: closed, one reachable minor (the abandon halted at its cursor) and smaller ones. Fix round 3 (92 rows, all killed); re-check part 3: closed, one minor (the abandon's footprint after a Stop), fixed by the orchestrator with a test and row C2-44.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan | 1 | 3 | `planned-state-read-as-actual` (M1: the deferred settlement kept a Stop's bound but not a loss's cutoff) — a plan defect |
| Sol, code | 2 | 1 | M1 `planned-state-read-as-actual` (a loss after `Provider::stop` never reached the waiting switch: the Stop's "played to the end" stood for a device lost before `b`); M2 `failure-collapsed-into-value` (a boundary send with no result left the final row `pending`) |
| Opus, re-check 1 | 0 | 3 + 1 new | new: `planned-state-read-as-actual` (a Stop with `B > b` settled `applied` before device time reached `b`; a later loss could not overturn it) |
| Opus, re-check 2 | 0 | 4 | `planned-state-read-as-actual` (the abandon halted at its cursor, a planned position, as if device time) |
| Opus, re-check 3 | 0 | 1 | — (a footprint that depended on the outcome) |

Every `planned-state-read-as-actual` finding here is inside the one state machine design note 49 (direction D′) made the answer to that mechanism: a nominal position (a Stop's bound, a cursor) read as what the device played. They were fixed in that state machine, not patched beside it; no new design note.

## Dispositions

**Sol, test plan (FAIL)** — all adopted: M1 `Halted { stop, played }`, a loss passes its sample, the deferred settlement uses it (UHD `stop_at(.., lost)` moved into C2-2); m2 two accepted switches settle each on its own boundary send (`[applied, uncertain]`, row C2-20); m3 H4 marked INFERRED; m4 the `Session.switch()` docstring.

**Sol, code (FAIL)** — all adopted, fixed in fix round 1.
- M1, `Streams::end` returned early once `Provider::stop` had ended the streams, so a later loss before `b` never reached a switch whose boundary send then completed whole (`applied` for a device lost before `b`): the loss after the end lowers `played` on a waiting switch (`Bookings::lose`); `c2_s22` both orders.
- M2, a worker panic (or a detached join) after issuing the boundary send left the switch in flight and the final Manifest row `pending`: **decided by the orchestrator** (an application of owner Q2's meaning "whether any of it was sent is unknown"), at the Manifest write an unresolved boundary send settles `uncertain` (`cancelled` if a Stop's `B ≤ b` came first); `c2_a_boundary_send_whose_worker_panicked_is_uncertain_in_the_manifest`.
- m3, the specs said an in-flight send always waits; C2-2's immediate cancel for `B ≤ b` is kept (the oracle's order) and the specs say so.

**Opus, re-check 1** — majors closed. New finding adopted, **decided by the orchestrator** applying owner Q1 (a loss settles by device time): a Stop with `B > b` after a whole boundary send keeps the switch waiting until device time reaches `b`; a loss at a sample at or before `b` first cancels it; the Manifest write applies it (fix round 2). Minors: RM-37 says the exception wins over a loss before `b`; note 49's row reworded; the oracle's `switch_outcome_after_loss` takes the halt's bound. Recorded, no change: the detached-join path has no test of its own; C2-24 is a staging-level contract row (one loss is published in production).

**Opus, re-check 2** — closed. Adopted (fix round 3): the abandon (UR-22) halts at device time, not at its cursor (a non-lost send error or a sequence-error report can abandon past `b` before device time reaches it); F8 on a held row gives `cancelled`, tested and stated; `settle_unresolved` applies through `apply()`; RM-37, UR-38 and MR-42 say a whole result under an earlier Stop waits and that the Mock settles at the Stop. Recorded, no change: the tie at exactly device time `b` (a Stop or `advance` at sample `b`, then a loss at sample `b` → `applied`; loss first → `cancelled`; from `advance`'s `b <= now`, predates C2); C2-32 and C2-38 near-duplicates.

**Opus, re-check 3** — closed. Minor, an abandon after a Stop left a waiting switch the Stop's bound, so its later apply refused `[s, stop_S)` while an applied-at-once or cancelled switch did not: fixed by the orchestrator, a waiting switch takes the abandon's bound (`c2_record_an_abandon_after_a_stop_keeps_its_footprint_whatever_the_waiting_switch_becomes`, failing before the fix; row C2-44 killed).

**Implementer's findings** — the `ideal` Mock panics in `TxBook::take` when a switch is received at the repeat's first sample (debug assertion; reproduced at `fbde789`, before C2): filed as #100. `n47s5_front_rehearsal_on_the_fake` failed once under full-workspace load in C2-4 (a real-clock Run rehearsal, note 51 stage 2c); not reproduced.

## Decided by the orchestrator

- Q3 (a Stop in flight settles by the result), Q6 (KL-42 kept as "Gone in C2" with its symptom corrected: the Mock had the same defect), Q7 (`Bookings::switch(.., carried)` explicit).
- C2-3 merged into C2-4 (C2-2 had taken its loss half); C2-5 retired three rows and retargeted three.
- The two Manifest rules above (M2: unresolved at the Manifest write → `uncertain`; re-check 1: a Stop's `applied` waits for device time, a loss before `b` cancels) — applications of owner decisions Q1 and Q2, listed for the owner at landing.
- No Python suite run in the fix rounds (only a docstring changed in C2-2); the gate runs them. `rules.py` and the gate take `--renames` for two renamed tests (`e2a_s11_…`, `e2a_f3_…`).

## Gate

PASS ([c2-gate.json](c2-gate.json)) on sim02:8 and sim03:8: 2511 tests, 619 UHD, Python 121 × 2, e2e 9/9, mutation 2326/2327 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, E2A2A-10, N51A-14, N51A-15, N51B-08, N51C-06, `phase7/G16`, no `ERROR`, no timing re-runs; `--renames` for the two renamed tests.
