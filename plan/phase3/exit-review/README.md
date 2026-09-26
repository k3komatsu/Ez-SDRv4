# Phase 3 exit review — per-rule OV-3 dispositions

One disposition for every rule Phase 3 introduced or amended: spec 11's
`CH-1…CH-11`, spec 12's `KB-1`, `KB-2` and every rule its `VB-1…VB-9` amend or
add, and spec 12's `M11…M15` process decisions. It follows the evidence rule of
[Phase 2's exit review](../../phase2/exit-review/README.md): a test citation is
included only when the test body asserts the obligation, not because the name
carries the rule prefix.

| file | document | rules |
|---|---|---:|
| [11.md](11.md) | spec 11 — the SimulationChannel (`sim` 1.1.0) | 11 |
| [12.md](12.md) | spec 12 — the amendments (`KB-1`, `KB-2`, `VB-1…VB-9`, `M11…M15`) | 36 |
| **Total** | | **47** |

## Dispositions

- `default`: the Phase 3 behaviour is asserted by the cited Phase 3 test body.
- `producer`: a Module-side obligation is carried by its Provider's tests.
- `process`: a document, spec text or step procedure is the carrier; the
  `read from` cell names that artifact.

No row is `GAP` or `UNCERTAIN`. A `—` in `test(s)` is used only for a structural
or process carrier, and the `read from` cell states it.

## What the exit review changed against Appendix B

`20-implementation-plan.md` Appendix B is the draft these tables were started
from. Three of its rows are superseded, and the reason is in each table:

1. **VB-7's "MR-13, MR-15, MR-16" row** named `mr_32_waveform_refusals` as the
   carrier of "the refusals' place before MR-17". That is false: the test's three
   bursts are on time and not repeated, so it cannot observe where the refusals
   sit. `mr_32_a_the_waveform_refusals_hold_their_place_in_handle_tx_burst`
   carries it, and the position is now killed at both ends.
2. **No row cited the tests Review C's fixes added.** Eleven unpinned rule clauses
   and six test gaps were closed after Gate P, and the carriers are listed here.
3. **MR-27 had no row at all.** VB-9 amended it during Review C, and
   `v58_08_b_two_mocks_in_one_run_keep_both_sets_of_sections` carries it.

Gate X acceptance and Step X move spec 11 to `design/` and apply the eight Vision
issues; the accepted decisions are recorded in `plan/phase3/00-overview.md` §11.
