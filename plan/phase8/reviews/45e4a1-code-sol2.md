# REVISE

No P1. One P2 remains.

## Prior findings
- **P2-1 — Closed.** `crates/ezsdr-radio-uhd/src/brain/core.rs:498–506` now retries after `Decide`. The two-stream regression asserts cancellation of A and reservation of B in that same step (`brain/bounds_tests.rs:465–473`; paths below share the same `src/` prefix).
- **P2-2 — Partly closed.** Done-before-Shutdown is covered, but queued boundaries/timers still suppress the required step-end check (`brain/oracle.rs:407–431,958–988`).
- **P2-3 — Closed.** B8 is explicitly amended (`plan/phase8/45-experiment.md:227`). The deterministic test covers F set → timer reserves B → S abandons without Apply → resident until terminal → retirement, and requests whole-sequence thread replay (`brain/bounds_tests.rs:480–528`).
- **Replay recommendation — Closed in code; execution unverified.** Acceptance is recorded at return in both wrappers (`brain/world.rs:697–700,1024–1029`; `brain/threads.rs:565–573`). The delayed-return regression checks both timestamps (`brain/thread_tests.rs:688–703`); witness and whole-walk replays reject every divergence (`:139–151,655–673`).

## Remaining finding
**P2 — P6’s blanket queue exemption still hides an omitted retry.**
`crates/ezsdr-radio-uhd/src/brain/oracle.rs:408–414,986–988`

Minimal sequence:
```text
K=1; A completed; B held and otherwise eligible.
inbox=[Done(A, now≥e_A), Shutdown, unrelated FirstSample boundary].
Process Done; incorrectly omit retry; finish its operations.
Process Shutdown → cancel B.
```
The boundary anywhere in the inbox disables step-end P6. Shutdown then removes B and disables subsequent P6 checks. Observable wrong outcome: B is cancelled without the reservation required at the freeing step. A queued capacity timer behind Shutdown gives the same escape.

**Not acceptable:** this preserves the adjudicated failure mechanism with one extra unrelated input. Check against the freeing step’s known capacity/eligibility rather than suppressing the check for every boundary/timer. Add both queue variants as negative regressions.

## Additional changes
- **I4 refinement is correct, not an improper weakening.** Return-time acceptance can legitimately credit a later sample. `applied.sample <= fact.sample` retains reservation/channel/epoch agreement and rejects pre-application evidence (`brain/oracle.rs:51–55,791–804`). Exact sample equality was too strong.
- **Instant-ordered releases correctly narrow the cases to production inputs.** Production selects the earliest held instant and floors subsequent bookings behind commands inside the release window (`provider/control.rs:648–673,690–712`; `design/18-uhd-radio.md:267–272`). The seeded dependencies and cases 11/12a remove impossible out-of-order release schedules, not a failure production must handle. Guard-refusal coverage remains in case 12b (`brain/cases.rs:437–465`).
- **P3 — Stale case description:** case 12a still says its cut is below A’s instant, although full uses A=8/cut=12 and micro A=1/cut=3 (`brain/cases.rs:422–430`). Correct the description.
- **400,000-state budget: acceptable.** Exhaustive completion remains mandatory; budget exhaustion is not accepted (`brain/oracle.rs:1293–1297`). The reported 302,471-state count was not independently reproduced.

Not checked: builds, test/mutation execution, 1–10,000 sweep results or witness counts, hardware, E4a2, or excluded holdout material. No files or git state changed.
