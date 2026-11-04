# Note 47 stage 3 — review record

Stage 3 (note 47 §4): receive frequency, gain and antenna change again, only on a stopped stream; the value is staged and used by the next `start_rx`; Provider refusals reach the submit reply (O2); `rx_streams[k].start(at, frequency=, gain=, antenna=)`; §58 #16's receive-antenna form. Implementer: Claude Sonnet subagents (test plan rev 1, rev 2, implementation with rev 3, fix rounds 1–2, gate fix). Reviews: Sol 6.1 (test plan, code), Claude Opus re-check reviewer (plan rev 2, code fix round). Briefs `47s3-brief.md`; test plan `47s3-test-plan.md` (rev 6); dispositions and rulings `47s3-dispositions.md`; reviews `47s3-sol-plan.md`, `47s3-sol-code.md`, `47s3-opus-rechecks.md`; gate `47s3-gate.json`; renames `47s3-renames.txt`.

## Findings by source and severity

| Source | Major | Minor | Mechanism (`plan/maintenance/failure-mechanisms.md`) |
|---|---|---|---|
| Sol, test plan rev 1 | 3 (scheduled updates cannot stage; the configure could eat part of the lead silently; `rx.start` never tested stopping on a Provider refusal) | 4 | none (plan gaps) |
| Opus, plan rev 2 re-check | 1 (an arrival-order "streaming" flag disagrees with the replayed line: Stop → start(at=+1 s) → Stop → set f2 accepted while the start still begins with f1) + ruling 7's seven plan changes | 1 | `stream-lifecycle-scattered` (a second record of the stream's state), caught in the plan |
| Sol, code | 3 P2 (a booked future start revised by an earlier one leaves uhd-rx's cursor past the surviving segment; negative channel keys index from the end in Python; an immediate booking refusal without the Action id is missing from `Submitted.refused`) | 3 P3 (receive start-lead bench group not restored → named follow-up; S7 tests narrower than planned; spec errors) | P2-1 `planned-state-read-as-actual` (uhd-rx acted on a planned origin a later booking changed); P2-3 `correlation-by-partial-key` (a refusal without its Action id); P2-2 none |
| Opus, code fix re-check | 0 | 1 (MockRadio overflow ended the Run; pre-existing) + 3 nits + 2 leftovers (the `|| halted` start branch; a test that hung instead of failing) | none |
| Gate 1 | — | 3 rows (one compile error, two survivors after test rewrites) | none |

## Decisions (orchestrator)

- Ruling 7: the receive start lead covers the start's configure; the origin is fixed at booking; `radio.START_LEAD_SHORT` when the configure leaves less than `min_timed_command_lead_ns`. Rejected: moving the origin (re-planning after booking), the full-lead threshold (fires on every restart).
- Sol P2-1 fixed by refusing a `start_rx` on a stream that is streaming after replay (a booked start included), not by reconciling uhd-rx with a revised plan.
- The receive start-lead bench group returns with the bench step (note 47 work order ③), before any profile `rx_start_lead_ns` changes.
- `Line::halted` and its field are read only by tests now; left for stage 6 (shared machinery clean-up).
