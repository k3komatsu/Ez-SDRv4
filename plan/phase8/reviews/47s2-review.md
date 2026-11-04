# Note 47 stage 2 — review record

Stage 2 (re-staged, note 47 §4 「段階 2 以降の組み替え」, 「2a と 2b をまとめる」): every radio key classless, the Run-time change machinery deleted in both directions. Implementer: Claude Sonnet subagents (parts a, a-fix, b, c1, c2, d, fix, fix2). Reviews: Sol 6.1 (re-staging, test plans s2 and s2a, code), Claude Opus re-check reviewer (plan re-check s2a-2, part (a) re-check and its fix re-check, the c1 unkilled-row classification, the fix re-check). Briefs: `47s2-brief.md`; per-test decisions: `47s2-test-plan.md` (rev 8); mutation rows: `47s2-rows.md`; dispositions: `47s2-dispositions.md`; renames: `47s2-renames.txt`; gate: `47s2-gate.json`.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, re-staging review | — | — | (design input: the must-survive list) |
| Sol, test plan s2 (rev 1) | 2 | 5 | none (plan gaps) |
| Sol, test plan s2a | 3 | 4 | none (plan gaps) |
| Opus, test plan s2a re-check | 3 | 3 | none (inventory gaps) |
| Opus, part (a) re-check | 3 | 6 | none (deleted pins of surviving behaviour, restored) |
| Opus, part (a) fix re-check | 0 | 5 (+1 tightening) | none |
| Opus, c1 unkilled-row classification | 4 rows wrongly deleted, 15 owed pins, 1 gap (restart snapshot) | — | none (missing tests; c1's probe width too narrow) |
| Orchestrator (step 6) | 0 | 1 (TX clock end, first accepted) | — |
| Sol, code | 1 (P2: TX clock no longer ended when a loss or `Provider::stop` cuts its plan at its origin) | 1 (P3: spec text describing deleted machinery) | none |
| Opus, fix re-check | 0 | 1 (P3 clause) + 1 leftover (dead `timed_tune` basis) | — |

No finding is a design mechanism of `plan/maintenance/failure-mechanisms.md`: the majors are missing or deleted tests and one deletion that removed a still-reachable clock end (restored).

## Lessons

- The compiler and the test run are the inventory for a deletion; name heuristics deleted retained behaviour (s2a plan, M2).
- A probe that decides "no surviving test kills this row" must run every binary with a differential (Mock timeline, `ezsdr-radio` timeline), acceptance `v61`/`x3xx`/`reactive`/`uhd`, server `protocol`, and the row's `old2`; part (c1) ran fewer and deleted 4 live rows and hid 15 owed pins.
- "Dead" needs a search of every path that changes the plan (the TX clock end was reachable only through a loss or `Provider::stop` before the clock's origin).

## Gate

First full gate: every step PASS except mutation (1656/1674): 4 rows no longer compiled and 13 survived (timeline cases the rebalanced generator stopped producing, the Mock value history now with one value per channel, a Mock fault row whose Stop was booked after the fault, Python aliases with equal rates, the owner's configure-once comparison). Gate fix round: 7 retargeted or re-anchored, 8 owed tests added, 3 equivalent rows deleted (J02, J12, M66, replaced by P1400). Second gate: see `47s2-gate.json`.
