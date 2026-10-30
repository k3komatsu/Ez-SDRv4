# Stage D9b-2 — review record (2026-10-30)

Implementer GPT-6 Luna fast (#max): test plan, a run stopped before code on the drain/successor conflict, the code run, four fix rounds. Self-review: the brief named one free-DeepSeek round; the implementer instead ran Sol 6.1 three times with its own focus list (recorded below, not counted as the independent review). Independent review Sol 6.1 (test plan, code, three re-reviews); orchestrator Claude Opus. Owner decision during the step: [design note 39 §6.7](../39-dsp-timed-commands.md) (decision (A): a post-cut receive segment is ready no earlier than cut + release window + one receive call + allowance).

| Source | Stage | Severity | Finding | Mechanism | Disposition |
|---|---|---|---|---|---|
| Sol | test plan | P1 ×4, P2 ×3 | consumption shown by samples, not `e <= c`; handover vs stop ordering; drain bound vs the successor; host silence still consumes; recovery after a failed drain; restore cases; the existing UR-24 raw test | test plan (not a design mechanism) | all adopted before code |
| implementer | before code | blocker | the drain (up to e + packet + allowance) eats the successor's lead (20 000-sample packet at 1 MS/s, 3 ms window and lead) | — | owner decision (A), note 39 §6.7 |
| Sol (implementer's) | code | P1, P2 | a stale `stopped` flag turns a covered successor's timed retune into an early untimed write; a failed flush stop still restores | `mech:stream-lifecycle-scattered` ×2 | fixed (P1228, P1229) |
| Sol (implementer's) | re-review | P1, P2, P3 | completion vs a late hand-over race (lock released before completion is recorded); sequence 14 tested stable states only; raw P4 capture made asserting | P1 `mech:stream-lifecycle-scattered`; P2 test power; P3 — | fixed (lock through completion, P1231; raw P4 observation-only again) |
| Sol (implementer's) | re-review | — | PASS | — | — |
| orchestrator | code | note ×3 | the report said first segments and TX were delayed too (they are not); the implementer dropped `CARGO_TARGET_DIR` after a compaction and built inside the worktree (removed; the worktree's ignored `.cargo/config.toml` pins it since); Sol used instead of the brief's DeepSeek | — | report corrected; process notes |
| Sol | code | major ×2, minor ×2 | M1 a flush `recv` reuses its timeout per packet and runs past the drain deadline into the successor's lead (UHD 4.10 `rx_streamer_impl.hpp:154–182`); M2 a successor refused after a hand-over leaves the entry with no owner; m3 mutation rows did not pin the ordering claims; m4 RM-25 lacked its Changes row | M1 — (UHD recv semantics); M2 `mech:stream-lifecycle-scattered`; m3 test power | fix round 1 (P1232–P1237) |
| Sol | fix 1 | major ×2, minor | M1 a request capped at the packet size is still multi-packet (`one_packet=false`); M2 a successor cancelled in `await_release`, a source-refused move's destination and shutdown with no stream still skip the settlement; m P1234 copied the restore instead of moving it | M1 —; M2 `mech:stream-lifecycle-scattered`; m test power | fix round 2: `one_packet` through `Device::rx_recv`; one settlement every owner exit reaches (P1238–P1240) |
| Sol | fix 2 | major, minor | M `pending_settled` remembered that settlement ran, so an obligation handed after an empty settlement was never settled; m a far-future booked cut made every earlier read one-packet | M `mech:stream-lifecycle-scattered`; m — | fix round 3: the flag removed, settlement per obligation (P1241, P1242) |
| implementer | fix 3 | hang | the new far-future-cut test froze the manual host clock while `rx_recv` waited for packets (17 min at 0 % CPU; the orchestrator killed it by PID and redirected the implementer with `opencode2 run -s`) | test harness | fixed in the test |
| Sol | fix 3 | minor | that test runs `rx.turn()` on the plain test thread; a scheduling pause over ~100 ms reads as an overflow | `mech:wall-clock-test-oracle` | fix round 4: a registered `HostClock::spawn` participant |
| full gate | fix 4 | 1 compile error | P1215's match text changed in fix round 3 (`let entry = dsp.pending.remove(position);`), so its mutant no longer compiled | test power | retargeted at the gate; 1/1 killed; gate 1942/1943 (G09 known), workspace 2110, uhd 535 + 5, Python 3.14/3.9 57 each, rules 0 |

The FakeDevice P4 A/C test asserts the model's DDC log and which restart sample took the value, not the bench's RF verdict; the raw hardware probe stays observation-only. Hardware is not yet rerun. Not done in this stage: dirty-chain isolation after a failed flush (KL-9), the transmit side (D9b-1b).

## Sol, test plan

VERDICT: REVISE

### Independent failure scenarios

Before reading the implementation brief or test plan, I identified these cases:

- Several equal-time commands need separate transfers, on every affected channel.
- Control hands another value over while RX is deciding to stop.
- Device time advances, but no samples traverse the DDC.
- A logical cut ends publication, while physical draining must continue without releasing the streamer early.
- Drain latency delays the successor’s start or overlaps its configuration.
- A cancelled value is physically consumed; restoration must follow consumption and preserve the correct logical setting.

The plan covers the basic drain, multi-channel, loss and cancellation cases. It misses important boundary and concurrency cases.

### Findings

### 1. P1 — “Due by cut” does not establish consumption

**Plan:** lines 9, 25 and 32.

Sequences 3/10 assume commands at or before the cut need no drain. That is unsafe for equal-time commands at the cut, late handovers, or missing samples. At radio-rate RX, several commands at `e == c` can require transfers after `c`.

**Evidence:** UHD 4.10 `axi_tag_time.v:207–211` dequeues on a qualifying transfer; `ddc.v:118–142` consumes the tagged value. Neither tests whether wall time passed the instant.

**Change:** Make the no-delay control conditional on **established consumption**, not `e <= c`. Add:
- multiple commands at `e == c`, at radio-rate RX;
- a due-before-cut command handed over after its corresponding samples passed.

Assert every handed entry is consumed before stop, while publication still ends at the original cut.

### 2. P1 — No test orders concurrent handover against final stop

**Plan:** sequences 1–10 are effectively serial.

RX can observe an empty tracking set while Control is inside `apply`, then stop before Control publishes the new obligation. Tracking only `released_at` also risks losing an obligation when Control folds its row.

**Evidence:** `provider/control.rs:669–674` publishes `Given` after `give`; `:702–709` folds entries. RX independently stops and finishes (`provider/rx.rs:202–212`).

**Change:** Add gated interleavings around device acceptance/tracking publication and RX’s final stop decision. Require each accepted timed RX frequency either to join the old chain’s drain or to follow a safe later-segment path. Include an entry whose logical row has already folded. Add gain, deferred/untimed frequency and refused-apply controls: these must not create DDC drain obligations.

### 3. P1 — The drain deadline and successor-start contract are unresolved

**Plan:** lines 9, 29 and 31.

The fixed `cut + release_window + delivery_allowance` bound ignores packet delivery time, the existing quiet-span rule and receive calls already in progress. With a 20,000-sample packet at 1 MS/s, otherwise healthy delivery can exceed the proposed 6 ms bound. Merely keeping existing no-pending tests green does not test this.

**Evidence:** `provider/rx.rs:384–419,628–629`; `provider/control.rs:309,286` accounts for packet/call size when constructing the timeline.

**Change:** Specify the bound’s relationship to packet time, `quiet_span`, minimum receive timeout and late-discovered cuts. Add long-packet/delayed-delivery cases and a successor on the **same streamer**. Assert:
- no early `finish`, pruning or channel release;
- old draining ends before successor configuration/start;
- the successor retains its planned origin and its start is issued in time.

Also test a nondefault short restart lead. If the proposed bound cannot preserve these guarantees, return that conflict to the orchestrator before implementation.

### 4. P2 — Sequence 7’s `Silence` fault cannot produce its asserted leftover

**Plan:** line 29.

FakeDevice’s `Silence` suppresses host delivery; it does not stop DDC sample flow. Advancing device time and querying `dsp_pending` will therefore consume the entry, contradicting the proposed assertion.

**Evidence:** `device.rs:1729–1732`, versus `:1064–1095`: consumption follows `flows`, independently of host reads.

**Change:** Separate:
1. missing host evidence despite continuing DDC flow;
2. an actually stopped/stalled chain with an unconsumed entry.

Use a controlled device adapter or explicitly modeled flow interruption for the second. Add overflow handling separately; the current fake overflow advances the host cursor rather than reproducing the FPGA’s acquisition-ending overflow path.

### 5. P1 — Timeout leaves the dangerous state but defines no safe successor behavior

**Plan:** sequence 7 deliberately leaves `dsp_pending != 0`.

Recording a rejection does not prevent the next stream’s first samples from taking the stale value. Sequence 8’s restore is safe only after a successful drain; an untimed write cannot repair this leftover.

**Change:** Define and test what happens after bounded-drain failure. Do not silently resume the affected chain as clean. Either require recovery that demonstrably clears the state or explicitly block reuse until recovery. Record that exception to R2′ and the resulting user-visible limitation. The exact recovery choice needs orchestrator adjudication.

### 6. P2 — Restoration covers only the easiest successful case

**Plan:** sequence 8.

Restoring A after successful drain is sound in principle. But the plan does not pin the source/order of the restored value, per-channel restoration, failure behavior, or the record describing the physical repair.

**Change:** Extend sequence 8 with:
- no earlier A: restore the initial in-force value;
- multiple channels and equal-instant updates at `S`, respecting booking order;
- a failed/lost restore: no subsequent start may assume restoration succeeded.

Assert restoration against the original end instant `S`, not drain-completion time. Specify its record and observe device state **before another Run’s prepare/configure**, which could otherwise hide an incorrect restore.

### 7. P2 — Existing stale-state characterization was overlooked

**Plan:** line 51 says no current Module-path test hands a timed RX frequency and then cuts before its instant.

`provider/control.rs:7024–7048`, `ur_24_a_cut_booked_after_the_release_leaves_the_entry_for_the_next_segment`, already books that sequence, then manually performs the raw device stop/restart and asserts stale overwrite.

**Change:** List it explicitly. Preserve its raw-device characterization, clarify that it bypasses the RX owner, and add the complementary owner-path regression. Update its obsolete limitation wording and the corresponding UR-24 test description.

### Evidence limits

The cited FPGA verifies transfer-dependent consumption and untimed STOP behavior. It does **not** establish a universal release-window drain bound. P4 A/C verify stale-state persistence; the plan correctly avoids claiming FakeDevice parity for P4 B’s AUTO/AUTO split.

Read-only review: no edits, builds, tests or hardware runs. Exact deployed-image timing and recovery after failed drain remain unverified.

## Sol, code

VERDICT: FAIL

### Scenarios reviewed

- Handover racing with stop and segment completion.
- Equal-time entries, separate channels, pre-acceptance samples, overflow and missing delivery.
- Cut, abort, Provider stop, cold change/reopen, refused configuration/start, LateCommand and device loss.
- Drain/flush completion against the successor’s origin and minimum lead.
- Restoration at original stop instant S, including booking order and failed stops/restores.

### Major findings

### 1. A flush receive call can overrun its deadline and the successor’s lead

**Location:** `crates/ezsdr-radio-uhd/src/provider/rx.rs:343–355`; `crates/ezsdr-radio-uhd/src/uhd.rs:1045`.

The flush passes its remaining budget as `rx_recv`’s timeout, but requests `max(block_len, packet_samples)` samples. UHD does **not** treat that timeout as a whole-call deadline. With `one_packet=false`, it reuses the timeout for each packet until the requested buffer fills.

**Concrete sequence:** use 1 MS/s, `block_len=20,000`, packets of 1,000 samples, release window 3 ms and RX lead 3 ms. Let the old chain stall with an entry at `c+3 ms`. Recovery begins at `c+23 ms`, with deadline `c+26 ms` and successor origin `c+29 ms`. After `stream_now`, packets arrive every millisecond. Each arrives within the timeout, so the 20,000-sample receive can continue until approximately `c+43 ms`. The deadline check runs only afterward.

This violates decision (A), even though recovery’s new flow is healthy. FakeDevice also models successive packet waits (`device.rs:1783–1826`), but the new deadline test uses a single long packet and does not expose this case.

**Primary evidence:** UHD 4.10 [`rx_streamer_impl.hpp:154–182`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/transport/rx_streamer_impl.hpp#L154-L182).

**Fix:** make flush reads small enough not to accumulate multiple packet waits, and enforce the remaining deadline between reads. Add a deterministic stalled-old-flow → healthy multi-packet-flush regression asserting completion before the readiness floor, plus a mutation row.

### 2. A refused successor can leave handed entries without any draining owner

**Location:** `crates/ezsdr-radio-uhd/src/provider/rx.rs:447–482`; `crates/ezsdr-radio-uhd/src/provider/control.rs:828–891`.

Control may hand a timed frequency to a planned successor before its timed start. The existing test `ur_24_a_successor_frequency_after_its_origin_stays_timed_during_restart` explicitly exercises that ordering.

If the successor’s `rx_start` then returns a non-loss error, `begin` marks `dsp.stopped=true` and returns without creating `self.stream`. The pending entry remains. Subsequent turns skip that origin, and Provider shutdown exits when `self.stream` is absent (`rx.rs:228–239`). No `try_end`, bounded flush or `rx_drain` report handles the entry before cleanup closes the streamer.

A configuration refusal after an earlier handover has the same ownership gap: the segment is refused without creating an RX owner that drains its pending entries.

**Result:** a timed DDC value silently survives refusal and close/reopen—the failure this stage is intended to prevent. KL-9 describes **reported** bounded-recovery failures, not this unreported bypass.

**Fix:** retain responsibility for handed entries when successor configuration/start fails. Run bounded cleanup, or explicitly report its failure, before skipping the segment, releasing channels or closing the streamer. Extend the gated successor test with non-loss configuration/start refusal and inspect pending state and rejection records through shutdown/reopen.

### Minor findings

### 3. The mutation rows do not establish several promised properties

**Location:** `plan/maintenance/tools/mutations.json:15292–15500`.

The rows cover individual conditions, but do not cover all planned sequence claims. For example, P1223 is named “restores only after draining,” yet its mutation disables restoration; it cannot establish that restoration happens **after** draining. The implementation report also explicitly identifies missing dedicated rows for the owner-path drain, gated handover and loss sequences.

**Fix:** add focused rows that break the actual properties: abandon a pending obligation at stop, bypass handover serialization, restore before consumption, and continue recovery after loss. Pair them with tests observing those failures, rather than treating restoration’s absence as evidence about its ordering.

### 4. RM-25’s amendment lacks its Changes entry

**Location:** `design/07-radio-model.md:251`, `design/07-radio-model.md:474–511`.

RM-25 gains the post-cut release-window readiness term, but its Changes table contains no D9b-2 amendment linked to note 39 §6.7. The new row in spec 18 does not replace spec 07’s required record.

**Fix:** add the RM-25 Changes row linking the owner’s decision.

### Other checks

The shared timeline adds the RX-only term consistently in MockRadio and UHD. Reviewed expectation changes retain their previous comparison strength. TX production behavior is unchanged. Raw P4 A/C remains separate from RF-tone verdicts.

The separate untimed DDC register path is **VERIFIED** in UHD 4.10 `ddc.v:114–144`; it does not reset the timed FIFO.

No files changed; no builds, tests, mutations or hardware runs performed. The two lifecycle/timing defects above remain unverified by execution.

## Sol, fix1

VERDICT: FAIL

### Major 1 — A maximum-packet cap is not a one-packet receive

**Location:** `crates/ezsdr-radio-uhd/src/provider/rx.rs:815–819`; `crates/ezsdr-radio-uhd/src/uhd.rs:1045`.

`bounded_read_len` caps requests at `rx_packet_samples`, but UHD still receives with `one_packet=false`. That size is a **maximum**, not a guarantee that the first returned packet fills the request. Short packets can therefore cause multiple packet waits, each using the original timeout.

For example, with 3 ms remaining, a request of 1,000 samples can receive two 500-sample packets after successive 2 ms waits, returning after the deadline. This affects both flush reads and bounded non-flush reads.

**Evidence:** UHD 4.10 [`rx_streamer_impl.hpp`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/transport/rx_streamer_impl.hpp) returns after the first packet only when `one_packet` or another early-return condition applies; otherwise its loop reuses `timeout_ms`. The regression at `rx.rs:1337–1459` fixes packet size at 1,000 and checks request size. The old oversized request fails that assertion, but this does not establish an actual one-packet bound.

**Fix:** use a genuine one-packet receive for deadline-sensitive reads. Test short packets and late non-flush reads, observing completion time and successor lead.

### Major 2 — Shutdown can still bypass an ownerless obligation

**Location:** `crates/ezsdr-radio-uhd/src/provider/rx.rs:228–229,444–446`; `crates/ezsdr-radio-uhd/src/provider/core.rs:1423–1426,1478–1480`.

The new recovery handles configuration/start errors returned to `refuse_segment`, but not cancellation or disappearance of a successor before it acquires an RX owner.

**Concrete sequence:** a destination owner waits in `await_release`; control hands a timed frequency to its planned covering segment; Provider stop then removes that future segment. `await_release` returns `None`, `owner_configure` returns `Ok(false)`, and `begin` returns without recovery. With `self.stream=None`, shutdown exits immediately. The pending entry receives neither bounded cleanup nor an `rx_drain` report.

A move refused by its source can similarly remove the destination’s segment without invoking the destination’s `refuse_segment`.

**Fix:** settle pending obligations on cancellation/no-segment and shutdown paths before releasing or closing the streamer. Add gated stop-during-wait and source-refusal/destination tests.

### Minor — P1234 still does not prove restore-after-drain ordering

**Location:** `plan/maintenance/tools/mutations.json:15522–15529`; `crates/ezsdr-radio-uhd/src/provider/rx.rs:1245–1258`.

The test consumes all pending entries in `rx.receive(block)` before `try_end` reaches the mutated code. P1234 inserts an additional restore but retains the original restore. It therefore dies because restoration occurs twice—not because restoration preceded consumption.

**Fix:** test restoration while recovery still has an outstanding entry, observe consumption before the restore apply, and mutate by **moving**, rather than copying, restoration ahead of recovery.

### Notes

- P1233’s held-lock-plus-waiter assertion tests actual blocking. P1235 directly observes no receive/final-stop calls after loss.
- Refreshed J15/P248/P484/P521/P1211 and phase7 B01 each match exactly once and retain their intended targets.
- RM-25’s Changes entry is present at `design/07-radio-model.md:512`.

No files changed. Builds, tests, mutations and hardware execution were not run, as instructed; verification counts in the implementer’s report were not independently reproduced.

## Sol, fix2

VERDICT: FAIL

### Major 1 — Settlement can ignore a newly handed obligation

**Location:** `crates/ezsdr-radio-uhd/src/provider/rx.rs:381–391,468–473`; `crates/ezsdr-radio-uhd/src/provider/control.rs:854–858`.

**Problem:** `pending_settled` records that settlement once ran, not that the current obligations were settled. It becomes `true` even when the pending list is empty. Control can subsequently append an obligation without invalidating that flag.

**Concrete sequence:**
1. The old segment finishes; settlement sets `pending_settled=true`.
2. Before RX begins its planned successor, control hands that successor a timed frequency. `reach()` permits a timed write to a covering, uncompleted successor (`control.rs:780–790`).
3. Provider stop removes the successor before `begin()` selects it.
4. Both no-successor settlement and no-stream shutdown settlement return immediately because the flag remains true.

The accepted entry receives neither recovery nor an `rx_drain` rejection. The reset at `rx.rs:473` is unreachable when no successor remains.

**Evidence:** The new shutdown regression constructs a fresh RX owner (`rx.rs:1572–1585`), whose flag starts false. The awaited-owner regressions enter `begin()` first, resetting it. Neither exercises a new handover after an earlier settlement.

**Fix:** Make settlement deduplication track the obligations actually reported, or invalidate it atomically whenever an obligation is added. Add a gated regression covering the sequence above, including an earlier empty settlement.

### Minor 2 — A far-future cut changes ordinary streaming immediately

**Location:** `crates/ezsdr-radio-uhd/src/provider/rx.rs:262–263`.

**Problem:** `stream.cut.is_some()` means a cut is **booked**, not that the receive is after or near it. Booking a change far ahead immediately switches all intervening receives to one-packet delivery, including when no DDC obligation exists.

For example, with `block_len=20,000` and actual packets of 500 samples, ordinary delivery changes from 20,000-sample blocks to 500-sample blocks until the cut. This increases publication/allocation frequency fortyfold and reduces a fixed-block-capacity link’s sample buffering fortyfold. The report’s claim that ordinary streaming is unchanged is therefore false.

**Fix:** Select one-packet mode for deadline-sensitive receives, not merely for the existence of a future cut. Keep the post-cut bound, and add a regression showing a far-future booking preserves ordinary multi-packet delivery.

### Notes

- The flag is correctly forwarded through `Device::rx_recv` to UHD (`src/uhd.rs:1045`), and FakeDevice honors it (`src/device.rs:1827`).
- P1234 now moves restoration before recovery; its test retains an outstanding entry and checks receive-before-restore ordering and zero device-side pending entries at restoration.
- P1234, P1238–P1240, P269, P513, U40 and B01 each match their source exactly once and target the stated behavior. P1239/P1240 do not cover Major 1’s previously settled owner.

No files changed. No builds, tests, mutations or hardware execution were run; reported execution results were not independently reproduced.

## Sol, fix3

VERDICT: FAIL

### Minor 1 — The new packet-mode regression still depends on scheduling

**Location:** `crates/ezsdr-radio-uhd/src/provider/rx.rs:1586–1598`.

**Problem:** Releasing `Frozen` fixes the hang, but both `rx.turn()` calls run on the ordinary test thread, not a registered host-clock participant. The fake device clock therefore advances while that thread is descheduled.

**Evidence:** FakeDevice treats a receive more than 100 ms behind as overflow (`crates/ezsdr-radio-uhd/src/device.rs:836,1767–1770`). A pause exceeding approximately 102 ms between releasing the freeze and the first receive makes the expected 2,000-sample block absent, despite correct packet-mode logic. A similar pause after `spy.set_time(cut_at)` can break the second assertion. The independent Core manual clock does not prevent the fake’s host clock from advancing.

**Fix:** Run the receive sequence in a registered `HostClock::spawn` participant, with gates keeping it busy while the harness advances the clocks and inspects results. Release the initial freeze only after that participant is registered. Time should advance during packet waits, not arbitrary test-thread scheduling gaps.

### Notes

- **Previous major closed:** `rx.rs:384–389,421–429` deduplicates reports per obligation instead of remembering an earlier settlement. Pending entries remain available to deadline and restoration checks. `control.rs:7528–7580` covers an empty settlement followed by a real handover, successor removal, and repeated settlement.
- **Previous production minor closed:** `rx.rs:259–260` selects one-packet mode only when the read starts at or after the cut; flush reads remain one-packet. A pre-cut read remains the single ordinary call allowed across the cut.
- **Mutation targets:** P269, P1241 and P1242 each match exactly once and target their stated behavior (`plan/maintenance/tools/mutations.json:6374–6381,15592–15609`). P1242’s killing test needs the scheduling fix above.

No files changed. No builds, tests, mutations or hardware runs performed; execution claims in the implementer’s report were not independently reproduced.

## Sol at the implementer's request, code

1. **[P1] A successor’s timed retune can become an early untimed write.**  
   `crates/ezsdr-radio-uhd/src/provider/control.rs:838–839` converts `Reach::Timed` to `Untimed` whenever `dsp.stopped` is true. But RX clears that flag only **after** successor configuration (`crates/ezsdr-radio-uhd/src/provider/rx.rs:434–463`).  
   **Failing scenario:** pause the owner after configuring a successor at origin `O`, before issuing its start. Release a frequency due at `O + 1 ms`. The old segment’s stopped flag converts it to an untimed write, so the successor starts with the new frequency 1 ms early. If release precedes configuration, configuration can overwrite it entirely. Preserve timed delivery for the covered successor and add a gated restart regression; the existing just-after-origin test never exercises the owner’s stopped state.

2. **[P2] Failure of the flush’s final stop does not prevent restoration.**  
   `crates/ezsdr-radio-uhd/src/provider/rx.rs:354–358` reports a failed final `rx_stop` but returns normally. The caller then restores frequencies without checking `dsp.stopped` (`rx.rs:325–326`).  
   **Failing scenario:** during `Provider::stop`, the initial stop succeeds, the recovery flush consumes post-`S` value B, and the final stop fails without device loss. Restoration still writes A and records success while RX remains running, contrary to UR-26’s required skipped restoration. Propagate the final-stop outcome and test refusal of the **second** stop; the current refusal test covers only the first.

**Verdict: Request changes—fix both findings.**

Read-only review; no files changed or tests rerun. Hardware behavior remains unverified here; neither finding requires changing TX behavior or raw P4 characterization.

## Sol at the implementer's request, re-review

VERDICT: CHANGES_REQUIRED

### BLOCKERS

1. **[P1] Completion can still race with a late retune and create a stale DDC entry.**  
   `give()` obtains coverage through `reach()`, then separately reads active identity (`crates/ezsdr-radio-uhd/src/provider/control.rs:831–850`). RX releases the DSP lock **before** recording completion (`crates/ezsdr-radio-uhd/src/provider/rx.rs:333–335,503–515`).

   Concrete interleaving: the stopped segment is still active when `reach()` returns `Timed`; RX then records completion; control’s second snapshot finds no active covering segment. Consequently `active_covers` is false, the untimed downgrade is skipped, and control queues a timed entry after completion (`control.rs:850–864`). No owner remains to drain it.

   Determine coverage and active/completed identity from one consistent snapshot, or serialize completion with the handover decision.

### NONBLOCKING

2. **[P3] Raw P4 A/C characterization is not entirely unchanged.**  
   The extracted sequence now calls `read_until()`, which asserts that restart capture reaches its endpoint (`crates/ezsdr-radio-uhd/tests/common/d9.rs:154–166,226`). Previously the hardware probe returned partial/empty capture and printed its observations after the bounded read. A missing restart capture now panics before the explicit stop and result reporting (`common/d9.rs:227–230`).

   Preserve the raw probe’s observation-only capture behavior; keep model assertions in the FakeDevice test.

### TEST_GAPS

- **[P2, related to blocker 1] Sequence 14 overstates the exercised lifecycle.**  
  `d9b2-test-plan.md:38` describes stopping and finishing the RX owner. The test instead directly installs active/completed state and sets `stopped` (`crates/ezsdr-radio-uhd/src/provider/control.rs:4042–4074`). It covers both stable states, not completion between control’s two snapshots. Mutation P1230 removes the completed filter but does not exercise that interleaving (`plan/maintenance/tools/mutations.json:15482–15489`). Add a gated owner-completion regression.

### PREVIOUS_FINDINGS

- **Successor retune: closed for the reported scenario.** The regression preserves timed delivery and passes actual FakeDevice samples through `Rx::receive` before asserting owner-observed consumption (`crates/ezsdr-radio-uhd/src/provider/control.rs:4011–4038`). Sequence 12 matches those assertions.
- **Final flush-stop failure: closed.** Restoration requires `dsp.stopped`; the second-stop-refusal test asserts no restore apply, false stopped state, and skipped-restoration records (`crates/ezsdr-radio-uhd/src/provider/rx.rs:323–328,1418–1429`). Sequence 13 matches.
- **Completed-segment exclusion: partially closed.** Stable completed and active-but-stopped cases are covered; blocker 1 remains.
- Changed mutation anchors, including P1228–P1230, each match exactly once. No production TX behavior change found.

Read-only review; no files changed or tests/mutations rerun. Hardware behavior was not revalidated.

## Sol at the implementer's request, re-review 2

VERDICT: PASS

### BLOCKERS

None. No additional concrete race or regression found in the inspected lifecycle paths.

### NONBLOCKING

None.

### TEST_GAPS

No remaining gap in sequence 14’s completion-race coverage. Tests, mutations, and hardware were not rerun; reported results were not treated as independent evidence.

### PREVIOUS_FINDINGS

- **P1 — closed.** RX retains its per-stream DSP lock through `finish()` on normal completion, already-stopped, and lost paths (`crates/ezsdr-radio-uhd/src/provider/rx.rs:231–235,280–283,290–340`). Completion is recorded before unlocking (`rx.rs:509–521`).
- **Related sequence-14 gap — closed.** The gated regression exercises the owner’s stop/completion boundary, verifies control waits, then asserts no late device write or pending obligation (`rx.rs:1105–1144`). P1231 uniquely matches that boundary and drops the lock before the gate; it defeats the waiting assertion and permits the forbidden write (`plan/maintenance/tools/mutations.json:15492–15499`).
- **P3 — closed.** Raw P4 A/C uses nonasserting capture and proceeds to stop/report even with incomplete capture (`crates/ezsdr-radio-uhd/tests/common/d9.rs:160–172,228–235`). FakeDevice retains explicit A/C stale-entry and restart-consumption assertions (`crates/ezsdr-radio-uhd/tests/fake.rs:8686–8705`).
- **TX behavior unchanged.** `provider/tx.rs` changes are test fixtures only; the shared timeline’s new readiness term is RX-only (`crates/ezsdr-radio/src/timeline.rs:1069–1073`).

Read-only review; no files changed. Hardware behavior was not revalidated.
