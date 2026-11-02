## Verdict: REVISE

`TP` = amended `test-plan-e2a.md`; code paths below are relative to `crates/ezsdr-radio-uhd/src/`. TP §13 explicitly supersedes earlier text; superseded contradictions are not counted separately.

### Earlier findings and adopted dispositions

| # | Closed? | Evidence |
|---|---|---|
| 1 — observations and duplicates | **Partial** | `TP:247` adds evidence, leftovers, discarded counts, ordered facts and duplicate rejection. However, field-inequality tests do not test rejection of repeated identical records; leftover duplicate handling is unspecified. See N2. |
| 2 — missing witnesses | **Yes** | `TP:251` separates smoke from mandatory full coverage and makes missing witnesses fail before E2a closes. Existing `brain/cases.rs:630–635` already enforces unreached-witness failure. |
| 3 — production clock/Q1 | **Yes** | `TP:250` replaces the native read with a no-`L` time source and retains differences as findings. Confirmed by `provider/core.rs:1233–1234`, `authority.rs:240–251`. |
| 4 — locks/timers | **Partial** | `TP:248–249,255` distinguishes acquisition from pending attempts, requires actual lock logs and adds free timer coverage. Acquisition authorization remains unspecified (N1); E2-23 still lacks an executable clock-contention sequence (N3). |
| 5 — legal S3/S4 contracts | **Yes, at contract level** | `TP:252` corrects `Done{Abandoned}`, guard `BOp`s, and Failed versus Lost. Confirmed by `brain/world.rs:326–330,1097–1118`. Explicit runnable prefixes remain implementation-review work. |
| 6 — S7/S10 vacuity | **Yes, at contract level** | `TP:254` requires application/evidence ticks exactly `e`; `TP:256` requires a Stop inside the buffer, exact publication/discard results, and issue-versus-call observation. These supersede the permissive original assertions. |
| 7 — conformance | **No** | `TP:257` names the added scripts but supplies no actual expected results. It also names affected E1 cases without explaining why readiness/tail/timeout differences preserve their verdicts. See N4. |
| 8 — smoke and killers | **Partial** | `TP:253,258` fixes future-origin streaming witnesses, names controller/helper tests, shares recv-end decisions and requires unwind-safe restoration. S6b and several new killers still lack concrete setups/assertions (N3). Updated sizes appear at `TP:259`. |

### New or remaining findings

**N1 — P1: post-acquisition hooks do not authorize acquisition.**  
`TP:44,248` says the controller releases the next event, but the lock hook now fires only **after** acquiring `L`. No pre-acquisition permission boundary is specified.
- Minimal sequence: with `L` free, execute `SCheck`; S acquires `L` and parks at its post-acquisition hook; the legal trace next selects `RLock(0)` before `SLock`.
- Wrong outcome: R cannot acquire `L`; the controller cannot release S without violating the trace. A valid World trace falsely diverges or times out.
- World permits either acquisition while free (`brain/world.rs:501,515`). Specify controller permission **before** acquisition and completion acknowledgment afterward; test both acquisition orders. Preserve the distinction between a pending attempt and a completed event.

**N2 — P2: observation self-tests do not exercise duplicate rejection.**  
`TP:247` tests differing fields, not repeated identical writes. An extractor using a map could silently collapse two identical terminals while every proposed inequality test passes.
Add malformed raw-log tests requiring extraction failure for duplicate dispositions, terminals, qualifying evidence, and leftover consumption; define identity/uniqueness across the two consumption lists. Mutation killers must disable rejection, not merely remove a field from equality.

**N3 — P2: new killer names still substitute for observable sequences.**  
`TP:253,258` does not specify S6b’s forced debt/leftover setup, required flush/restore calls, or cleanup end state. Likewise, “E2-23’s trace is in S8” adds no `ClockLock/Read/Unlock` steps to `TP:158`.
Write the actual contention sequence and the direct `consume_samples` test: first offer, persisted `seen`, second offer, exact expected outputs. State controller-invalid-test inputs and expected rejection points. Until then, E2-23/E2-28 and cleanup coverage are not implementation-ready killers.

**N4 — P2: conformance expectations remain promises rather than scripts.**  
`TP:257` says each script states its expected result, but none does. Specify concrete times/channels and expected return/application/evidence results for persistence, reversed command instants, and before/after acceptance. Device-only `Step` records (`TP:166`) cannot themselves observe wrapper consumption: identify the wrapper fixture or consumption-rule invocation used by the acceptance test. Provide case-specific justification before waiving readiness, tail or timeout differences.

Not checked: builds, test/mutation execution, hardware, implementation correctness beyond cited connections, or the prohibited holdout directory.
