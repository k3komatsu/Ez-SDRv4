# #110 + #113 — dispositions and counts

#110: an admitted start-batch Action dropped by an end before its dispatch was recorded nowhere (RS-16). Owner decision (2026-11-19): record it in `termination.undelivered`, ActionIds assigned at admission; the batch is admitted atomically. Sixth instance of `mech:pending-provenance-collapsed`: design note [n110-note.md](n110-note.md) — the batch moved into `Shared.start_batch`, drained by the one freeze like every other pending holder. #113: a MockRadio receive segment cut at or before its origin with no clock ended at the stop instant (owner: at the origin, RM-25/MR-25); `mech:radio-rule-implemented-twice` (the stray path aligned; the UHD Module has no such row). Way of working: a Sonnet subagent (Go DeepSeek for sweeps and tests); Sol 6.2 reviewed the test plan and the code; Opus and GPT-6.1 Terra reviewed the code side by side; plan v2 went to code without a paper re-check. Briefs [n110-briefs.md](n110-briefs.md), test plan [n110-test-plan.md](n110-test-plan.md) (v2), reviews [n110-reviews.md](n110-reviews.md), reports [n110-reports.md](n110-reports.md). Gate: `n110-gate.json` (beside this file).

User-visible: a dropped start batch's ids appear in `termination.undelivered` (also after a failed `start()`); a Module `Abort` submitted between arm and dispatch gets id n+1, not 1; a MockRadio receive segment cut at or before its origin ends at the origin; `ActionId`'s description (schemas regenerated, no structure change).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Sol 6.2, test plan v1 | 4 | 4 | 0 | test-plan gaps (none counted) |
| Opus, code a53a8917 | 0 | 4 | 5 | none |
| Terra, code a53a8917 | 0 | 2 | 1 | none |
| Sol 6.2, code d4f840ff..cecf798b | 1 | 2 | 1 | none (F1: a test that could not see the second push) |
| Terra, re-check | 0 | 1 | 0 | none |

## Decisions (orchestrator unless marked)


- Owner 2026-11-19 「推奨で」: #110 records an admitted start batch dropped before dispatch in `termination.undelivered`, ActionIds assigned at admission; #113 `ended_at` = origin (RM-25/MR-25).
- Mechanisms: #110 `pending-provenance-collapsed` (6th; design note note.md: merge the start batch into the Shared pending holders drained by the one freeze); #113 `radio-rule-implemented-twice` (13th; the stray path aligned).
- Plan (test-plan.md, Sonnet) questions: Q1 accepted — a batch admitted before a failed `start()` (Failed{arm}) is in `undelivered` (it was admitted and not delivered; the owner's decision as written); the two tests named are updated. Q2 accepted — `start_probe::Point::Admitted`.
- Visible numbering change (plan §2): a Module's `Abort` submitted between arm and dispatch now gets id n+1, not 1 (the batch is always 1..n); no fixture or golden carries ActionIds. Within the owner's decision (ids at admission); reported at landing.

## Sol 6.2 test-plan review (sol-plan.md): FAIL, 4 majors, 4 minors — dispositions (orchestrator); all adopted, plan v2 then code without a paper re-check
- F1: M2 books the Stop in a step strictly before the origin (the unregistered branch); the registered case stays in its existing test.
- F2: a test with an end between two pushes (A delivered, device loss at `Dispatching { next: 1 }`, B pending): the hooks establish the freeze reaching the gate while dispatch holds admission; B is either pushed before admission is released or drained — the Manifest's `undelivered` holds exactly the ids not received by the Provider; receipt + Manifest assertions kill the row, waits as infra backstops.
- F3: J195 stays at the Abort call site with its inequality assertion (not retargeted to the allocator).
- F4: the start batch commits atomically: a refused entry consumes no ids (the cleanup-time Abort gets id 1); KC-17 says the batch is admitted as a whole; the test captures that Abort's id.
- F5: the numbering text is relative to a successful batch admission (a Module Abort can take an id while the schedule is being checked); the sweep's wrong claim fixed; the server and Python correlation tests cited.
- F6: S1 includes a far-future agenda Stop; its handle is cancelled once under the freeze gate with no admission or state guard held.
- F7: S4 keeps the `p:action:` line and records the id alongside a distinguishing value; S8's empty-batch assertion goes in an existing deterministic test that reads `undelivered`.
- F8: `ActionId`'s description (event.rs and the regenerated schema) no longer says "dispatched"; #113's UHD evidence cites the FakeDevice `rx_stop_untimed` cut == origin test and the TX clock-end test.

## Review of a53a8917: Opus (opus-code.md) PASS, 4 minors 5 notes; Terra (terra-code.md) FAIL, 2 minors 1 note — dispositions (orchestrator); one small fix round, then Sol
- Terra 1 + Opus 7 adopted: add the plan's "one id fewer" row (killed by S5 or the two-entry test) and the "stores no ids" row; record the row renumbering against plan v2 §4 in the report.
- Terra 2 + Opus 4 adopted: `ActionId`'s description covers a Module's `Abort` ("issued by the Run's counter: an admitted Action, or a Module's Abort"); regenerate the schemas.
- Opus 1 adopted: design/07 cites `ur_26_a_cut_at_a_queued_origin_stops_the_stream_at_once` (and the TX clock-end test) instead of the timing-row name.
- Opus 2 adopted: KC-18's boundary: "an end requested before the check that follows the publication of `Running`".
- Opus 3 adopted: KC-24a's restatement of RS-6 step 1 includes moving the start batch's undispatched ids, and that the dispatch empties `start_batch` under the admission lock.
- Opus 5, 6 adopted: the dispatch loop's comment names the admission lock; `debug_assert_eq!(ids.len(), batch.len())` before the `zip`.
- Opus 8, 9: the kc_14 message corrected; the gain-entry helper and the one `format!` where cheap; `dispatch` may stay on its own `fetch_add` (noted).

## Sol 6.2 code review of d4f840ff..cecf798b (sol-code.md): FAIL, 1 major, 2 minors, 1 note — dispositions (orchestrator)
- F1 adopted (major, test strength): the loss-between-pushes test (S9) also asserts the second Action reached its queue — the final `RunHandle::effective()` gain is 2.0, or the second push is observed before the freeze — so a loop that stops after the first push and leaves the second id to the freeze fails; plus a row for that mutant (stop after the first push), killed by assertion. Re-check: Terra alone (test-only).
- F2 adopted: the early-end watched-cancel test asserts the complete lock record at `cancel` (only the freeze gate held, `start_batch` and every state lock released).
- F3 adopted (text): design/07's citation states the shared cut rule and MR-25's MockRadio `ended_at` guarantee; UHD's record guarantee is narrower (no stop row when the device is lost before the origin) — say so.
- F4 (server disconnect reaches a child after its startup pipeline): pre-existing, noted.
