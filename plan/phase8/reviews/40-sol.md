# 設計メモ 40：Sol 6.1 のレビュー

## テスト計画のレビュー 1（`sol-plan`）

VERDICT: FAIL

### Independent scenarios

Identified before reading the test plan:

- Queue A and earlier B before any hand-over, versus B arriving after A was handed over.
- Finite A with its final sample held, versus its device EOB already sent.
- A buffer crossing `now + W`, including non-aligned sample lattices.
- B exactly at the first unhanded sample, and one sample before/after it.
- Stop followed by B while A’s handed samples remain queued.
- Cold-clock transitions, repeated preemptions, moved-start collisions, and independent streams.
- Zero-window behavior and profile-value propagation.

### Findings

**P1 — major: D2 mistakes a reservation for completed hand-over.**  
**Where:** note §2 D2, §3 sequences 7–8; test plan lines 26, 59–60, 77–78, 91.

A held A inside the window is not necessarily already handed over. UHD drains queued commands before stepping TX (`crates/ezsdr-radio-uhd/src/provider/tx.rs:136–150`), then selects the earliest held target (`:349–376`). Queue A at +4 ms and B at +3 ms before any TX step: UHD can transmit B first, on time. D2 instead declares B late. The planned UHD tests explicitly hand A over first, so they do not test the Mock sequence they claim to match.

**Fix:** distinguish booked from handed state. Add both interleavings through production booking/dispatch. Either emulate a defined hand-over progression or explicitly justify a conservative Mock envelope; do not describe every held reservation as already handed.

---

**P2 — major: `now + W` is not UHD’s actual front, even after hand-over.**  
**Where:** note §2 D2; test plan lines 26, 45, 71–79, 129.

UHD checks a buffer’s **start** against W, then sends a whole chunk (`crates/ezsdr-radio-uhd/src/provider/tx.rs:319–323,385–395`). At 1 Msps with `block_len=2,000`, now=0 and A starting at +9 ms, its first chunk ends at +11 ms. B at +10 ms is therefore late against UHD’s cursor, but on-time under D2. The existing Stop test even mentions the extra buffer (`:3502–3504`), although its aligned setup hides it.

**Fix:** decide whether W is a strict bound or a send-start threshold. For a strict bound, clamp sends at the lattice boundary and test actual hand-over; otherwise model/document the overshoot. A pure ceil test cannot establish hardware-path equivalence.

---

**P3 — major: sequence 11’s exact front and “fully handed” setup ignore the held tail.**  
**Where:** note §3 sequence 11; test plan lines 11, 81, 127.

The deviation is real in principle: with `open=None`, `dispatch_start` checks time/origin/lead, not earlier queued payload (`crates/ezsdr-radio-uhd/src/provider/tx.rs:579–585`). However, the proposed +2-to-+8 ms finite burst does **not** hand over all samples: `send` retains its last sample when the end is more than L away (`:405–413,435–438,497–502`). At 1 Msps the first unhanded sample is +7.999 ms, not +8 ms.

Moving B to +8 ms can be a defensible conservative rule, but it is not the stated exact first-unhanded rule. A genuinely fully ended device burst also cannot safely restart at exactly its end: UR-23 documents the measured gap-zero failure (`design/18-uhd-radio.md:246–247`).

**Fix:** split held-tail and EOB-already-sent cases. Assert actual device samples and boundary metadata, not just the Kernel EOB record. Specify the legal restart/continuation boundary for each.

---

**P4 — major: lifecycle and isolation claims have no discriminating tests.**  
**Where:** test plan lines 93–100, 127–131; note §2 D2.

Existing cold-change and multi-stream regressions do not establish the lifetime of the **new front state**. Missing sequences include:

- A handed ahead → Stop → B inside A’s remaining queued interval.
- Old-clock front → cold switch → B on the new clock.
- Stream 0 front → B on otherwise idle stream 1.
- Front expiry → later B with no predecessor.

Mock Stop clears `open` and `held` (`crates/ezsdr-mock-radio/src/lib.rs:1142–1161`); UHD Stop cannot recall sent samples (`crates/ezsdr-radio-uhd/src/provider/tx.rs:521–551,694–701`; RM-16). Recomputing only from surviving bursts loses that boundary.

**Fix:** add controlled-clock tests for these transitions, including reservation cleanup and unchanged other-stream behavior. Resolve open items 1, 2 and 5 before implementation.

---

**P5 — major: the proposed existing-test update has incorrect sample arithmetic.**  
**Where:** test plan line 91.

`moved_duplicate` creates A with **two samples**, not 2 ms: `tx_action` takes a sample count, and the call passes `2` (`crates/ezsdr-mock-radio/tests/mock_radio.rs:51,939`). Its end is sample **2,003,002**, not **2,005,000**. At receipt sample 2,001,000, the proposed D2 move would be to 2,003,002 and lateness from target 2,000,000 would be **3.002 ms**, not 5 ms.

Also, both Actions are queued before one Mock step (`:939–941`), making this an important P1 interleaving rather than merely an obsolete assertion.

**Fix:** correct units and retain a genuine moved-start collision test instead of replacing its purpose with successful transmission.

---

**P6 — minor: the W=0 UHD runtime test has no viable transmission expectation as written.**  
**Where:** test plan line 80.

With W=0, a future held burst cannot satisfy `start − now < window`; it is dispatched only after its start passes (`crates/ezsdr-radio-uhd/src/provider/tx.rs:353–360`), when device-lead revalidation makes it late. “No extra lateness” is therefore not an executable UHD expectation.

**Fix:** keep the positive custom-W reader test; restrict zero-window behavior to Mock ideal unless UHD zero-window scheduling is deliberately added.

### Shared-function assessment

Centralizing interval reduction is reasonable, but `tx_preemption_front(window_end, spans)` does not resolve P1–P4. UHD already owns an observed cursor; treating it as reconstructed spans adds no useful sharing. Share only the settled boundary/decision arithmetic, and keep hand-over ownership explicit.

D1’s propagation and evidence labeling are otherwise sound. Open item 4 correctly leaves the measured W choice to the bench.

Read-only review; no files changed, builds, tests, or mutations run. Hardware outcomes beyond the cited bench remain unverified.

## テスト計画のレビュー 2（`sol-plan2`）

VERDICT: FAIL

### Previous findings

References below are to revised `test-plan.md`.

| Finding | Status | Evidence |
|---|---|---|
| P1 | **Partly fixed** | Lines 50–57 distinguish queued-before-dispatch from already-handed A, but D2′ still equates equal timestamps with no intervening hand-over; see Q1. |
| P2 | **Partly fixed** | Lines 80–89 test actual clipping, including a fractional sample lattice. Stop padding and its restart boundary remain unsettled; see Q2. |
| P3 | **Partly fixed** | Lines 75–78 separate held-tail continuation from completed EOB and inspect device samples. The stopped-repeat variant remains uncovered; see Q2. |
| P4 | **Partly fixed** | Lines 93–100 add lifecycle/isolation sequences. D4 lacks future-held and medium-output coverage; the expiry oracle is ineffective; see Q3–Q4. |
| P5 | **Fixed** | Line 120 correctly preserves `moved_duplicate`: two samples, end 2,003,002, same-step receipt. |
| P6 | **Fixed** | Lines 34 and 74 replace UHD W=0 behavior with a positive custom-W test. |
| Shared-function note | **Fixed** | Line 46 shares boundary arithmetic only; each Provider retains ownership of its hand-over state. |

### Contract assessment

**D2′ is not yet a sound equivalence to UHD.** Receipt timestamps do not identify the TX owner’s dispatch boundaries. It must either model a defined hand-over progression or explicitly describe a conservative envelope without admitting cases hardware rejects.

**D3’s payload clipping is compatible with held-tail continuation**, provided the deferred tail is sent exactly once and boundary calculations distinguish payload cursor, continuation position, and completed device EOB. Its nonaligned arithmetic is correct: at 390,625 sps, sample 3,906 precedes 10 ms and sample 3,907 is at 10.00192 ms. “Strict” therefore needs the stated lattice-rounded meaning.

**D4 fits RM-16’s ahead-of-time exception**, but contradicts current MR-25 and MR-32 text. Those implementing amendments and observable radiation tests are missing.

### New findings

**Q1 — major: equal receipt timestamps do not mean no hand-over occurred.**  
**Where:** note 40:52; plan:40,54–55.

**Problem:** Freeze manual time at `t`: book repeat A at `t+4 ms`, drain its command and step TX until samples are handed over, then book B at `t+3 ms` without advancing time. UHD must judge B late. D2′ excludes A because its receipt instant equals B’s, allowing Mock B on time.

**Evidence:** `provider/tx.rs:136–150` drains the queue **per loop**, not per timestamp. Nothing prevents multiple loops at one manual-clock instant. The revised earlier-hand-over test explicitly advances time, hiding this distinction.

**Fix:** Add this third interleaving. Define the emulation’s hand-over boundary independently of timestamp equality; merely changing `<` to `<=` would break the queued-together case.

**Q2 — major: Stop’s padded EOB needs a different restart boundary.**  
**Where:** plan:89,97,138–140.

**Problem:** Stop a repeat after payload has reached exclusive cursor H. With no held tail, UHD’s empty EOB adds a zero sample at H. Moving B merely to H is unsafe; applying the finite-burst `end+1 sample` rule is also insufficient because the padded device burst ends one sample later.

**Evidence:** `tx.rs:540–549`; UR-23 (`design/18-uhd-radio.md:246–247`) explicitly says empty EOB extends the burst and makes a burst one sample after the original end late. The existing Stop test expects ACK at `H+1 sample` (`tx.rs:3535–3540`).

**Fix:** Settle payload-window versus physical-padding bounds before implementation. Add stopped-repeat → B boundary cases, checking padding, timed SOB, actual samples and reports. At 1 Msps, distinguish H, H+1 and the safe positive-gap restart after the padded end. Do not silently tighten the accepted padding exception or reuse finite-tail boundaries.

**Q3 — major: D4 coverage stops at records, not the complete Mock behavior.**  
**Where:** plan:36,97,102–116.

**Problem:** The sole D4 sequence starts with a running repeat. It misses prior-received A starting at `s+4 ms`, already inside the emulated window when Stop arrives at s. It also cannot catch correct BurstRecords paired with radiation incorrectly truncated at s.

**Evidence:** Mock `stop_tx` separately emits blocks, cuts medium segments and cancels held bursts (`lib.rs:1142–1161`). MR-25 requires the old instantaneous cut; MR-32 also cuts radiation at that instant (`design/09-mock-radio.md:181,207`). Neither amendment appears in line 36’s inventory.

**Fix:** Specify/test Stop during future-but-handed A, including cancellation versus playback. Amend MR-25/MR-32 and assert loopback samples through the delayed cut, with no samples afterward.

**Q4 — minor: expiry test cannot kill its claimed mutant.**  
**Where:** plan:100,139.

An expired H is earlier than current time; retaining it forever still yields `max(current lead, H) = current lead`. B remains on time, so the proposed test passes that mutant.

**Fix:** Assert actual state removal if cleanup is required; otherwise drop the unsupported expiry-mutation claim.

Read-only review; no edits, builds, tests or mutations. Hardware behavior beyond the cited bench evidence was not revalidated.

## コードレビュー（`sol-code`）

VERDICT: FAIL

## Independent scenarios

- A finite burst’s device EOB is already sent; B requests its exact exclusive end.
- Stop drains a future burst; another Stop arrives before that drain finishes.
- A short send accepts part of a buffer; recovery closes it, then B targets the accepted interval.
- A held SendAsap burst is decided repeatedly around another reservation.
- Fronts remain separate across streams and clocks; fractional-rate rounding, repeat wraps and window-edge waits preserve progress.

## Findings

### F1 — Major: Mock still admits an exact-end restart that UHD rejects

**Where:** `crates/ezsdr-mock-radio/src/lib.rs:1385–1391`

At 1 Msps, freeze time at zero. Receive finite A, one sample starting at sample 4,000. Let UHD hand A over, then receive B targeting sample 4,001 under DropAndFlag.

UHD sends A with device EOB immediately: the one-sample exception disables tail holding (`crates/ezsdr-radio-uhd/src/provider/tx.rs:429–432`). It remembers safe restart sample 4,002 (`:517–522`), so B is late.

Mock still holds A before playback. Its finite contribution is only A’s exclusive end, 4,001. B equals that bound and is admitted. Mock is therefore looser than UHD, contrary to Vision §59 and RM-15 (`design/07-radio-model.md:166`).

The rejection in `report.md:86` is not justified. Ordinary lead does **not** hide this gap: both targets remain about 4 ms ahead, exceeding the 2 ms lead. Distinguishing Kernel EOB from device EOB is correct, but does not resolve this sequence.

**Fix:** Amend D2'' and Mock’s conservative front to account for device EOB that could already have been sent, including one-sample bursts and finite-tail deadline closure. Add paired Mock/FakeDevice exact-end and end-plus-one tests. Keep ideal’s zero-window behavior unchanged.

### F2 — Major: A second Stop recalls the first Stop’s handed samples

**Where:** `crates/ezsdr-mock-radio/src/lib.rs:1180–1186`

Extend `mr_25_stop_hands_a_future_repeat_through_the_in_flight_window` with another TX Stop at the same instant.

The first Stop records A’s 6,000 samples and preserves radiation from +4 ms through +10 ms. It then clears `open` and `held` (`:1194–1205`).

The second Stop excludes the remembered closed front, finds no active candidate, and chooses the current stop sample as its cut. `cut_segments` then removes the future A segment because its start is at or after that cut (`:944–951`). Its record still says 6,000 samples, but the medium radiates none.

UHD’s second Stop cannot recall the samples already handed over. This violates D4 and MR-25/MR-32.

The existing medium test checks only one Stop (`crates/ezsdr-mock-radio/tests/mock_channel.rs:644–667`). The explicitly requested running-repeat medium case is also absent; the added test covers a future repeat.

**Fix:** Preserve already-handed radiation when subsequent Stops cancel remaining work; do not recut the completed handed prefix at the new receipt instant. Test two Stops for both running and future repeats, checking records and medium samples.

### F3 — Major: Recovery remembers the cursor from before a partially accepted send

**Where:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:465–480,587–594`

When a send accepts only part of its buffer, `abandon` reads `open.next` before accounting for those accepted samples. It remembers that stale cursor plus two.

Concrete sequence at 1 Msps, frozen time zero:

1. A starts at sample 4,000; its first 2,000-sample send returns 1,000.
2. The device has accepted payload through sample 4,999. Recovery’s empty EOB adds padding at 5,000; safe restart is 5,002.
3. The new code remembers **4,002**.
4. B targets 4,500. It passes ordinary lead and the remembered front, but FakeDevice rejects its timed start inside the previous accepted interval.

FakeDevice explicitly supports this partial acceptance (`crates/ezsdr-radio-uhd/src/device.rs:1541–1542,1560–1561`) and rejects such starts (`:1585–1597`).

The report acknowledges missing recovery/restart coverage (`report.md:49,100`). Removing surviving P657 does not establish correctness; that missing sequence hides a real cursor error.

**Fix:** Carry the accepted sample count into recovery’s cursor. When an error leaves acceptance unknown, use a conservative attempted-send bound. Add short-send/failing-send → restart tests that observe padding, actual B samples, timed SOB and asynchronous errors; then restore an effective mutation row.

### F4 — Minor: Valid test citations disappeared from the specifications

**Where:** `design/18-uhd-radio.md:239,251`

The replacement Test lines omit existing, relevant tests without replacing their specific coverage:

- `ur_21_an_untimed_send_is_on_time_after_its_delivery` — asserts 20 completed bursts without time errors (`crates/ezsdr-radio-uhd/tests/fake.rs:1456–1465`).
- `ur_23_a_burst_alone_is_ended_before_it_runs_dry` — asserts acknowledgment without underflow (`:2641–2651`).
- `ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time` — asserts the actual sample and no time error (`:2819–2833`).

These behaviors remain normative and the tests remain present.

**Fix:** Restore these citations.

## Assessment

The aligned/fractional window tests, receipt-order tests and finite-tail continuation test observe meaningful device samples and metadata. The new mutation rows target real decisions, but they do not cover the finite-EOB parity hole or recovery cursor above.

**Not checked:** No files changed; no builds, tests or mutation runs executed, as requested. Hardware gap sufficiency outside the documented measurement remains unverified.

## 修正 1 回目の見直し（`sol-fix1`）

VERDICT: FAIL

### G1 — major: Stop can preserve an unhanded payload sample beyond the window

**Where:** `crates/ezsdr-mock-radio/src/lib.rs:315–318,1177–1191,972–980`; `crates/ezsdr-mock-radio/src/channel.rs:91–98`.

**Problem:** The safe **restart** front is also used as the payload cut. F1 can put that front beyond the strict window, letting Stop retain radiation that it never transmitted or recorded.

**Evidence:** At 1 Msps, freeze receipt time at zero, with horizon sample 10,000:

1. Book B at sample 10,000 first.
2. Book one-sample A at sample 9,999.
3. Stop at the same receipt instant.

Both bookings pass: B starts exactly at the horizon, so it contributes no front when A arrives. At Stop, A contributes **10,001** through the new one-sample exception. `transmit_until_cut` emits A, then opens B because `10,000 < cut`. B cannot emit: its sample is at the exclusive horizon (`emit_tx_until`, `lib.rs:1796–1797`). Nevertheless, `cut_segments` retains B with end 10,001. The medium therefore radiates B’s sample at 10,000, although no B block or burst record contains it and Stop cancels its unstarted open burst.

UHD cannot hand B’s payload at that exclusive window edge. This violates D3, RM-16 and MR-25’s cancellation of unhanded work. Existing repeated-Stop tests do not cover a held successor at this edge.

**Fix:** Separate the payload preservation cut from the safe restart front. Retain restart gaps for later admission without using them to open or preserve unhanded payload. Add this receipt-order/window-edge case, asserting records, cancellation, medium silence at B, and paired FakeDevice sends.

### G2 — minor: O2’s duplicate-error fix misses `preempt`

**Where:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:699–713`; `crates/ezsdr-radio-uhd/src/provider/control.rs:1151–1159`.

**Problem:** `dispatch_start` suppresses a repeated SendAsap event, but `preempt` still emits it unconditionally. The new UR-21 sentence is therefore false for an already-late burst entering this path.

**Evidence:** At 1 Msps and frozen time zero, hand repeat A, starting at 3,000, through cursor 7,000. Book SendAsap B targeting 1,000. Control moves B to 2,000 and emits its first `TIME_ERROR`, storing `open.late`. Preemption then moves B to cursor 7,000 and emits another SendAsap `TIME_ERROR` at line 710.

`ur_21_a_send_asap_move_is_reported_once_across_handovers` exercises repeated `dispatch_start`, not control → preemption, so it misses this case.

**Fix:** Apply the existing already-reported check in `preempt` too. Add the sequence above and assert one event, preserved requested target, and B’s actual continuation samples.

### G3 — note: Other fixes and rejected findings

- **F1:** Exact-end/end-plus-one tests observe lateness and actual B starts; the old Mock calculation would fail them. G1 is an adjacent failure introduced by mixing that restart boundary with Stop’s payload cut.
- **F2:** Repeated Stop now preserves the closed front. The new running/future-repeat tests check both records and medium samples and would expose the original recall bug.
- **F3:** Accepted counts now reach recovery. First-buffer short/error tests and continuation-tail tests observe fronts and device samples. Rejecting review-4’s “tail counted twice” claim is correct: the continuation cursor already follows the held tail, and successful tail delivery does not advance it again.
- **F4/O1:** The named UHD citations and ownership/counting descriptions are restored.
- **Review-5 finite-tail Stop concern:** `+2` is not unconditional. A completed finite burst takes the `ends` branch and retains `end + 1`; only a still-open burst takes Stop’s `open.next + 2` path. No demonstrated looser boundary there, though its dedicated paired regression remains absent.
- The shared late-policy helper preserves the three policies and equality boundary. W=0 rejection is justified for the current UHD profile. Missing dedicated mutation and closed-front isolation cases remain coverage gaps, not demonstrated additional failures.

**Not checked:** Read-only review at HEAD `d6ee79a`; no edits, builds, tests or mutations executed. Hardware restart-gap sufficiency beyond the documented measurement remains unverified.

## 修正 2 回目の見直し（`sol-fix2`）

VERDICT: FAIL

### H1 — major: Stop loses the empty-EOB restart gap at a held continuation

**Where:** `crates/ezsdr-mock-radio/src/lib.rs:1192–1198,1813–1868`.

**Problem:** G1’s payload cut is now separate, but this exposes an adjacent admission bug: a Kernel burst ending at a held successor clears `open` before Stop calculates `open_front`. Stop therefore misses the empty-EOB `+2` restart boundary.

**Evidence:** Extend the new window-edge regression at 1 Msps, frozen time zero:

1. Book B at 10,000, then one-sample A at 9,999.
2. Stop. The corrected payload cut is 10,000.
3. Book C with `DropAndFlag` at 10,001.

A ends at `held_cut`, so `emit_tx_until` clears `open` without remembering a closed front. Stop consequently has no `open_front`; its precomputed `safe_front` is only 10,001. Mock admits C at equality.

The paired FakeDevice test already demonstrates that this sequence leaves A’s **device** burst open and Stop sends an empty EOB (`crates/ezsdr-radio-uhd/src/provider/tx.rs:2607–2614`). `end_open` remembers cursor 10,000 **plus two** when no tail is held (`tx.rs:546–556`). UHD therefore rejects C at 10,001; its first safe start is 10,002. Mock remains looser than UHD, violating Vision §59.

**Fix:** Preserve the device-open continuation boundary through Stop even when the Kernel burst has ended. Keep its empty-EOB restart front separate from the payload cut. Extend the paired regression with C at 10,001 and 10,002, asserting rejection/admission and actual samples. Cover a repeat ending at the same held boundary too.

### H2 — minor: UR-23’s EOB statements contradict the new regression

**Where:** `design/18-uhd-radio.md:247–248`.

**Problem:** The spec says a one-sample burst carries EOB on its only buffer, and that only a repeat stopped mid-waveform with nothing held ends through an empty buffer.

**Evidence:** `ur_23_stop_does_not_send_unhanded_window_edge_payload` explicitly asserts the opposite for a finite one-sample A: its send has no EOB because B is adjacent, and Stop closes it with empty-EOB padding (`tx.rs:2607–2614`).

**Fix:** Qualify the one-sample statement with the already-booked continuation exception, and include a cancelled continuation after a completed finite Kernel burst among empty-EOB cases.

### H3 — note: Original G1 and G2 fixes are supported by the code

- **G1 payload leakage:** `stop_cut` now preserves previously handed payload independently of restart gaps. The new Mock test checks A’s record, B’s cancellation and medium silence; reverting the payload cut to the safe front would fail it. Repeated Stop retains the earlier payload cut.
- **G2 duplicate event:** Both dispatch and preemption now use `report_send_asap`. The new preemption test observes one event, the original requested target and B’s actual continuation sample.
- Cold-change and loss paths bypass Stop’s retained payload cut and cut at their own instant. The demonstrated remaining failure is the held-continuation restart boundary above.

**Not checked:** No edits, builds, tests or mutations executed, as instructed. Findings are from source inspection; hardware restart-gap sufficiency beyond documented measurements remains unverified.

## 修正 3 回目（判断 D5）の見直し（`sol-fix3`）

VERDICT: PASS

### J1 — note: D5 resolves the reviewed framing failures

**Where:** `crates/ezsdr-mock-radio/src/lib.rs:1378–1414,1863–1868,1172–1194`.

**Evidence:** Booking uses the handed payload end plus two samples for nonzero windows. Normal closure and held-continuation closure retain that same margin; the old one-sample and continuation exceptions are gone. Stop separately preserves `max(stop sample, handed payload end, previous Stop cut)`, then retains `cut + 2` for admission.

This addresses F1/H1 without restoring G1’s payload leakage or F2’s repeated-Stop recall. W=0 suppresses the reservation front. Cold change and loss keep their instantaneous cuts; retained state is SampleClock-qualified. No additional looser-boundary case was found within these modeled sequences.

**Fix:** None required.

### J2 — note: The regressions observe the relevant behavior

**Where:** `crates/ezsdr-mock-radio/tests/mock_channel.rs:671–805`; `crates/ezsdr-mock-radio/tests/mock_radio.rs:1061–1124`; `crates/ezsdr-radio-uhd/src/provider/tx.rs:2587–2635`.

**Evidence:** The H1 pair checks H+1 rejection and H+2 admission. Mock assertions cover burst records, cancellation and loopback samples; FakeDevice assertions cover payload, empty-EOB padding, absence of the rejected send and the new device burst. Repeated-Stop tests check both records and retained radiation.

P627, P645, P646, P652, P662, P670 and P672–P674 now target behavior their selected tests distinguish. In particular, P674’s repeat-Stop filter avoids the independently remembered normal-closure front that masked its original mutation. The retained logs support the reported nine kills.

**Fix:** None required.

### J3 — note: Spec changes and self-review dispositions are supported

**Where:** `design/09-mock-radio.md:145,182`; `design/18-uhd-radio.md:246–250`; `/home/komatsu/.cache/ezsdr-fixes/2026-10-25-preempt/fix3-report.md:49–61`.

**Evidence:** MR-17 states the conservative margin and Module-specific equality boundary; MR-25 separates payload preservation from restart admission. UR-23 now includes reserved-continuation cancellation among empty-EOB cases. These changes remain normative text with supporting Test lines.

The rejected release-guard concern supplies no reachable failure: current callers establish positive send counts. Keeping internal-state unit assertions is reasonable alongside the external regressions. Deferring orchestrator-owned plan updates and unrelated mutation cleanup is justified. The self-review suggestion to restrict the continuation exception to one-sample continuations would be wrong: `ends_at_held` also suppresses EOB on an initial one-sample burst.

**Fix:** None required for this round.

**Not checked:** No edits, builds, tests or mutations were executed. Unconditional hardware parity remains unproved: dispatch latency is explicitly deferred in `plan/phase8/40-preemption-bound.md:79`, and restart-gap sufficiency beyond the measured rate still requires bench validation.
