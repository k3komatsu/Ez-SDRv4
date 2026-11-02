## REVISE

`TP` below means `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan-e2a.md`.

### Findings

**P1-1 — `Observed` cannot detect incorrect consumption evidence or duplicate settlements.**  
`TP:96–114`, `TP:156`
- Minimal sequence: `apply(v) at sample 8; recv [8..11]; wrapper attributes consumption to sample 9; send Consumed(v)`.
- Wrong outcome: the wrapper claims evidence inconsistent with the device application, violating E1’s I4, yet dispositions, terminals, publications, applications and calls can all compare equal. No evidence identity or consumption tick exists in `Observed`, although S7 asserts one.
- Likewise, collecting dispositions/terminals into maps can hide repeated identical writes unless extraction explicitly rejects duplicates.
- Record qualifying evidence and leftover consumption separately, including reservation/sample identities; preserve multiplicity or assert uniqueness before collection. Include discarded-sample counts for S10. Add negative observation tests—not only the proposed `calls`-field self-check.

**P1-2 — S1 permits missing witnesses to pass.**  
`TP:141`
- Minimal sequence: `generate seeds 1–300; reach one common witness; never reach lifecycle-loss/late-completion witness; skip missing traces; report S1 PASS`.
- Wrong outcome: an incorrect wrapper on the omitted path receives no replay, while the experiment appears to satisfy “every declared witness.”
- Require the complete declared witness set. A short default sweep may be a smoke subset, but cannot satisfy S1(b); the wide run must be required before closing E2a. Missing witnesses must fail or explicitly leave the criterion unmet.

**P2-3 — Q1 rests on an incorrect production-clock claim and adds an unmodelled operation.**  
`TP:7`, `TP:217`, `TP:226–231`
- Production’s `self.core.now()` calls the time authority (`src/provider/core.rs:1233–1234`); `DeviceTime::now` extrapolates (`src/authority.rs:240–247`). It is **not** a synchronous `Device::time_now()` taking native `L`.
- The proposed post-apply device read introduces another lock/read/unlock sequence absent from the Event table. It can block behind another actor, not merely produce a different acceptance tick.
- Re-adjudicate Q1 using the actual production path. Specify acceptance recording and its replay boundaries explicitly. Retaining and reporting genuine differences is correct; treating this new native call as production-equivalent is not.

**P2-4 — Lock events and timer delivery need a consistent replay contract.**  
`TP:33`, `TP:44`, `TP:57–68`, `TP:158`
- DST `SLock`/`RLock` mean successful acquisition; the proposed hook precedes acquisition. “Blocked” counts as quiet, but must not count as completion of that event.
- S8 requests `RLock` while S holds L, although that event is disabled in `World` (`src/brain/world.rs:570`). Distinguish a pending lock attempt from the trace’s acquisition event.
- Also test `S holds L; K waits for ClockLock; drain Timer becomes due`. Forced replay delivers Timer from the controller, whereas free execution gives both jobs to K. A blocking clock task must not prevent a timer interleaving that DST permits.
- Add assertions on actual acquisition/release, and a free-run test for this clock/timer sequence. E2-19/20/23 need these observation points, not merely hook labels.

**P2-5 — S3 and part of S4 do not describe legal E1 sequences.**  
`TP:148–150`
- S3 expects no `Done`, but `abandon_command` explicitly enqueues `Done { terminal: Abandoned }` (`src/brain/world.rs:338–344`). A bare `Pub(Brain, Set)` is also not the brain guard’s Event representation: guard operations are `BOp`.
- S4’s `RFault → lost` is incorrect: RFault produces `CallResult::Failed`; actual device loss produces `Lost` (`src/brain/world.rs:1187–1208`).
- Build these tests from legal prefixes with explicit gate ownership, complete the result/send steps, and assert the actual contract. Otherwise E2-04/07/08 can fail for setup or divergence unrelated to their intended defect.

**P2-6 — S7’s late-application assertion does not reject late application; S10 need not discard anything.**  
`TP:156`, `TP:162`
- S7 accepts `application_tick >= e`, which permits the very late application it says must fail. Call-log order relative to publication does not establish that the call happened before device instant e.
- Require the controlled ratio-one case’s application/evidence tick to equal e, with separately observed admission/acceptance timing.
- S10 buffers 12..15 with cut 16 but names no Stop shortening the retiring cut. Publishing the whole buffer can therefore be correct, leaving E2-13 alive.
- Explicitly shorten the retiring cut into that buffer and assert exact published and discarded ticks; distinguish command issuance from its later device call for I12.

**P2-7 — Conformance can waive differences that change E1 verdicts.**  
`TP:166–170`
- The scripts omit cross-stream/channel persistence: `queue value on ch0; stop unread; close/reopen or move; receive again`. This is needed to validate leftover identity and I15, not TX fidelity.
- Include later-instant-then-earlier commands and consumption before versus after acceptance, especially given Q1.
- Packet readiness, partial-tail delivery and timeout returns affect eligible evidence and ordering. They cannot be blanket “listed differences” without showing that the tested E1 cases remain valid.
- Specify expected results, not only cross-device equality. The native-lock cases necessarily remain model-only; say clearly that FakeDevice conformance does not validate L.

**P2-8 — Real-clock smoke setup and mutation killers are incomplete.**  
`TP:154`, `TP:179–205`
- S6 starts at absolute tick 0 on a running real clock. FakeDevice treats past starts as late (`src/device.rs:1195`); use a future origin and prove both streams actually produced samples before releasing the value.
- S6 makes flush/restore optional and shuts down after consumption, so it does not force those paths. Keep this as a happy-path smoke and name separate non-vacuous cleanup coverage.
- E2-16/17 need explicit invalid-controller self-tests; successful replay alone does not demonstrate rejection. E2-23 names a K/L contention trace absent from S8’s sequence.
- E2-28’s retained `u12*` tests exercise `consume_packet`, not necessarily the newly moved `consume_samples`; give the new helper a direct duplicate-offer killer.

### Independent scenarios → planned coverage

| Scenario written before reading TP | Coverage assessment |
|---|---|
| 1. Blocked apply, deadline, stop | S8, partial: acquisition and free timer path need revision. |
| 2. Stale recv across cut/epoch | S1/S10, partial: require a genuinely shortened retiring cut. |
| 3. Delayed result versus receive evidence | S1/Q1, insufficient: acceptance boundaries and evidence observations missing. |
| 4. Loss check windows and lifecycle admission | S3–S5, partial: repair legal sequences and add the ordinary-call window case explicitly. |
| 5. Panic, stranded work, running device | S5/S1, partial: assert pending-command outcomes and retained/stopped device state, not exits alone. |
| 6. Hooks acknowledge labels, not operations | S1/S8, insufficient until acquisition/completion semantics are fixed. |
| 7. Wrong identity, duplicate or omitted evidence | S1, insufficient: P1-1. |
| 8. Matching empty runs | S1 witness skipping permits it by path; S6 needs streaming witnesses. |
| 9. Packet/tail/late promotion | S9a–c, useful, but timeout/readiness differences need bounded acceptance criteria. |
| 10. FIFO order and persistent channel state | S9d/e cover capacity; persistence and reordered instants are missing. |
| 11. Partial application reported as error | ModelDevice/Q2 cover injected partials; unknown non-loss errors still need an explicit decoder test. |
| 12. Forcing supplies missing synchronization | S5–S8 free variants help; add the blocked-clock/timer sequence. |

### Scope, maintainability and size

The extraction list is appropriately broad, and leaving FakeDevice unchanged matches the accepted split. However, `World::r_end` still contains loss/late-result decisions; name their shared implementation, not merely a sequencing pair. Keep oracle evidence independent of the shared publication decision.

The ≈2,400-line estimate is candid but optimistic: full `Device` decoration, forcing cancellation and richer observations need accounting. The accepted split is sensible; update the per-part totals and checkpoint after extraction. Scratch-swapping cells needs unwind-safe restoration; do not add a generic cell-set abstraction merely for anticipated performance.

Not checked: builds, test or mutation execution, hardware, or the prohibited holdout directory.
