My independent failure scenarios, before reading the E2a brief/design/test plan:

- **Framing:** a failed first send, held-tail send, or continuation leaves the device burst open, closes it twice, or emits the replacement waveform without the correct SOB/EOB. A dropped burst must not leak its remaining samples.
- **Lock/device split:** Stop, switch, loss, or a new booking arrives between planning a send and recording its result. Completion must settle exactly the planned booking once, without erasing later receipts or sending beyond the admitted cut.
- **Scoped abandon:** abandoning A must preserve an unrelated B—including B’s switch, busy interval, and overlap protection—even when B shares A’s accepted chain. Conversely, A’s cancelled switch must disappear from both held and unread payloads.
- **Unidentified reports:** an underflow after the device burst closed must not cancel a later booking. A report during a new burst cannot safely be attributed to an old booking merely from receipt order; test the chosen current-burst rule explicitly.
- **Switch receipt:** test before the predecessor’s first issue, during that issue, after issue but before completion, exactly at a boundary, and after the cursor passed one. Refusal must be visible in the submit reply; an accepted switch must never move later.
- **Chained switches / moved repeats:** after A→B, a B→C receipt must use B’s actual started state and grid, not A’s cursor or a stale `playing.origin`.
- **Hardware regression:** preserve sample values, timestamps, channel counts, SOB/EOB, held tails, padding, and reopen order—not merely device-call names. The short-first-send and rate-dependent close guards make apparently harmless trace changes consequential.
- **Parity/API:** distinguish the deliberate Mock/UHD “started” difference from accidental divergence, and check the new refusal against EA-11/EA-16 and Python’s documented switch failures.

The bench sections support these hazards; I have not independently verified their raw measurements.
Two plan expectations conflict with the current rules: the scoped abandon extends the old repeat’s chain into the separate Stop region, which changes SendAsap admission; and the timed hardware switch trial asks 40 ms ahead, so its planned `requested_target = at` assertion appears wrong. I’m checking the mutation killers and receipt/Stop sequences before the verdict.
## Verdict: FAIL

The TxBook direction remains sound. The plan needs corrections before implementation; **D4’s accepted equivalence premise is refuted by the code and should return to the orchestrator/owner.**

References below use `E2a/` for `/home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/`. **VERIFIED** means source inspected; predicted outcomes are **INFERRED**, not executed.

### Major findings

1. **The scoped-abandon rule changes SendAsap admission in F3.**

   **VERIFIED:** `E2a/test-plan.md:58-64` sets the abandoned repeat’s `front = s` and also adds `[c,s)` as a Stop region. Today, after an applied switch at `b=c`, the old repeat’s front stays at `b`; `halt` removes the successor and adds the separate region (`crates/ezsdr-radio/src/staging.rs:475-501,588-593`).

   **Minimal sequence:** R plays → S is applied in the record at `b`, but its payload remains unsent → abandon at `c=b` → submit timed SendAsap H at `b+1`.

   **INFERRED wrong outcome:** the proposed extended R chain overlaps H. `region_move` sees that chain and does not move H; admission refuses it instead of moving it beyond the Stop region (`staging.rs:430-436`). P10’s promised equality therefore cannot hold.

   **Fix:** preserve the distinction between the old chain and the separate Stop region. Pin both pending and already-applied cases. R9’s proposed `t7a3` setup is insufficient: that rig runs only through 8 ms, before its 14 ms boundary (`crates/ezsdr-radio-uhd/src/provider/tx/tests/o1.rs:770-780,857-865`), so it does not exercise F3’s already-applied state.

2. **The hardware rehearsal prescribes the wrong timed `requested_target`.**

   **VERIFIED:** `E2a/test-plan.md:271` requires `Some(at)` for the existing timed trial. But that trial requests `tx_at(now + 40 ms)` (`crates/ezsdr-radio-uhd/tests/common/n47s5.rs:270-273`; `tests/common/mod.rs:923-926`), comfortably beyond the receipt bound of W + lead.

   **Minimal sequence:** R starts → wait 20 ms → request timed S 40 ms ahead → accept S at `grid(at)`.

   **INFERRED wrong outcome:** a correct implementation records `None`, and the proposed rehearsal fails—or implementation is changed to satisfy the wrong expectation.

   **Fix:** retain this trial as the **on-rule → None** case. Add a separate deterministic pushed-timed case expecting `Some(at)`, plus untimed → None. This corrects the audit, not the settled #86 decision.

### Minor findings

3. **R4 reverses FIFO cancellation.**  
   `E2a/test-plan.md:111` receives Stop **then books H**, yet expects H cancelled by that Stop. H must survive; that is the explicit `TxList` contract (`crates/ezsdr-radio/src/staging.rs:239-242,268-272`). Book H before Stop for the cancellation case, and retain an after-Stop survivor assertion.

4. **Several named mutation killers do not exercise their stated mutation.**
   - **TailShort cursor row** (`E2a/test-plan.md:129`): the named continuation test stalls send **14**, the payload. Tail failure is send **13**, covered by `ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned` (`tx.rs:4265-4296`).
   - **Switch `started` row** (`:242`): R16 deliberately never issues S1, so omitting S1’s started update leaves its expected refusal unchanged. The proposed secondary killer, `ur_21_s9_switch_different_length_sets_the_new_period`, does exercise the positive path (`tx.rs:7268-7280`); use it explicitly.
   - **N47S5-90** (`:234`): its rewritten killer refuses the switch before successful booking (`:193`), so it cannot kill a mutation confined to successful `book()` handling of switches. Reassess whether that mutation remains reachable or meaningful.

5. **`calls()` comparison misses some attempted framing changes.**  
   **VERIFIED:** FakeDevice forces EOB false for short sends before logging; zero-accepted failures omit timestamp and stream (`crates/ezsdr-radio-uhd/src/device.rs:1829-1857`). Thus byte-identical traces do not prove identical attempted EOB framing on those paths. Supplement with entry-side attempted-send metadata; keep existing sample assertions.

### Checks that hold

- **D1:** the base-tree production ownership premise holds: held-map writes are in uhd-tx helpers; `open` and `chain` are Tx-owned. I found no receipt-side writer. This does not establish the broader claim that separate sections need no concurrency tests.
- **D2/D3:** Provider-specific started semantics and action-id matching are appropriate. Add the positive chained-switch case to R16 itself.
- The listed move/install test deletions are justified once receipt-taking and the unread guard land. The finite-abandon property remains pinned by P13.
- The EA-16/Python refusal wording and separate KL-41 entry are covered. No additional `design/` or KNOWN_LIMITATIONS omission identified.
- No concrete new maintainability defect beyond stale/ineffective mutation coverage.

**Not checked:** builds, mutations, raw bench archives, or implementation performance. No files changed.
