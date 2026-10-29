# Stage D9b-1a — review record (2026-10-29/30)

Implementer GPT-6 Luna fast (#max), one code run and four fix rounds; self-review free DeepSeek V4.1 Flash (code), Go DeepSeek V4.1 Flash (fix round 1, free at its limit); independent review Sol 6.1 (test plan, code, four re-reviews); orchestrator Claude Opus. Design decision during the step: [design note 39 §6.6](../39-dsp-timed-commands.md) (third strike and beyond of `mech:correlation-by-partial-key`).

| Source | Stage | Severity | Finding | Mechanism | Disposition |
|---|---|---|---|---|---|
| implementer | test plan | blocker | `prepare` configures TX untimed before `arm` opens the streamer; who reads the flush ack under UR-25 | — | Sol resolved: untimed configure stays in prepare, the opener reads the ack inside its claim |
| Sol | test plan | P1 ×2, P2 ×3 | TimeError tick is current time (never consume another burst's target); barrier on every open route; total flush deadline and failure cases; fault the closing EOB itself; T3 does not drain a pending timed value | test power / plan (not a design mechanism) | all adopted before code |
| self-review | code | P1, P2, P3 | UR-22 said every sample-bearing send is timed (the flush is not); recovery front inside the flush; stale wording | — | fixed |
| orchestrator | code | major | `recover_flush` spins at real-time priority when its claim is held or on Stop | — (removed by §6.6) | fixed: recovery is a streamer reopen; a deferred reopen sleeps |
| orchestrator | code | major | a late burst dropped without an ACK stays pending; later ACKs retire the wrong burst | `mech:correlation-by-partial-key` | §6.6 tick rule |
| orchestrator | code | note | `continuation_md` dead after T1 | — | removed (F9) |
| Sol | code | major ×3, minor ×5 | M1 flush debt by channel swallows or mistakes ACKs; M2 fake lacks idle-only time check and packet policy; M3 fake acknowledges before playback; m4 ACK after the deadline accepted; m5 B18 wrong test binary; m6 failed payload EOB without `tx_close`; m7 UR-13/UR-23 contradict T2/T3; m8 barrier test bypasses `Control::give`, probes share sequences but not verdicts | M1 `mech:correlation-by-partial-key`; M2, M3 `mech:device-state-outlives-call` (fake ↔ X3x0) | fix round 1 (§6.6 for M1) |
| self-review | fix 1 | P2 ×2, P3 ×4 | stale RM-15 `H + 2`; missing rows; fake model text; fault text; mechanism ledger; test gaps | — | fixed |
| Sol | fix 1 | major ×3, minor | M1 a report-triggered failed close lets the same pass send before recovery; M2 an ordinary Stop erases recovery; M3 fake checks lateness on the host time, not the surviving DUC time; m4 `late_reported` survives a generation | M1–M3 `mech:device-state-outlives-call`; m4 `mech:correlation-by-partial-key` | fix round 2 |
| full gate | fix 2 | 3 survivors, 2 baseline failures | P1158, J293, B21 survived (T2/T3 removed what they guarded); P1181, P1182 failed unmutated under shard load | test power; `mech:wall-clock-test-oracle` for the baselines | fix round 4: J293, B21 re-aimed; P1158 recorded equivalent; P1181/P1182 on the manual clock |
| Sol | fix 2 | major ×2 | M1 radio rejection freezes the fake's DUC; M2 a failed reopen clears recovery | M1 `mech:device-state-outlives-call` (third fake-fidelity finding: the fake's TX path rebuilt as a DUC stage then a radio stage); M2 — | fix round 3 |
| Sol | fix 3 | major ×2 | M1 a failed reopen leaves the burst stopped at the send barrier unsettled; M2 an untimed SOB keeps `has_time` in the fake | M1 —; M2 `mech:device-state-outlives-call` | fix round 4 |
| Sol | fix 4 | — | PASS | — | — |
| full gate | fix 4 | 3 survivors | P1177 (barrier helper took a waiter count as proof the gate was held), P1187 (the test's first one-sample EOB was untimed), P1201 (idempotent after the DUC rewrite) | test power, not a design mechanism | fix round 5: tests strengthened, P1201 retargeted; 3/3 killed; gate 1910/1911 (G09 known), workspace 2083 tests, uhd 509 + 5, Python 3.14/3.9 57 each, rules 0 |

The FakeDevice D9 probes (P5 A/C, P6, P7, D70) assert model-level outcomes — DUC logs, ACK/TimeError ticks, transmitted samples — not the bench's RF tone verdicts; `report-d9b1a-fix3.md` lists what each asserts. The production code meets the bench in the D9b-1a rerun.

Not done in this stage (D9b-1b, D9b-2): #71's TX release rule, TX R2′, owner choice (a) after an underflow, the receive side. Minor left: `tests/common/measurement/h4.rs`'s empty-EOB filter (P1158) is dead after T2.

## Sol, code

VERDICT: FAIL

## Scenarios reviewed

- Timed first sends, continuations, held tails and every closing shape.
- Short/failed EOB, recovery flush, then another burst on the same streamer.
- All three open routes racing timed DSP issuance and shutdown report reads.
- Missing, delayed and per-channel acknowledgments; unrelated reports; Stop and loss.
- Repeated late packets after starvation, stale headers across reopen, and surviving DSP commands.
- TimeError ambiguity, acknowledgment retirement, differential probes and mutation coverage.

Normal T1 sends and successful T2 closes look consistent. Recovery and FakeDevice fidelity still have blocking defects.

Paths abbreviated below: `src/` and `tests/` are under `crates/ezsdr-radio-uhd/`.

## Major findings

### 1. Recovery acknowledgment bookkeeping corrupts later burst correlations

**Where:** `src/provider/core.rs:1167–1172,1225–1236`; `src/provider/tx.rs:1037–1056,5086–5094`.

**Problem:** Flush debt identifies acknowledgments only by channel, not by the operation acknowledged. It can swallow an experiment acknowledgment or accept an experiment acknowledgment as completion of T3.

A concrete sequence is A’s short EOB, successful corrective close, then recovery flush. The corrective-close acknowledgment is consumed as “flush debt,” while A deliberately remains in `unacked`—the new recovery test explicitly asserts this. B’s subsequent acknowledgment then retires A instead of B. Similarly, if a timed-out flush acknowledgment was genuinely lost, B’s acknowledgment is swallowed as the missing flush acknowledgment.

The `BTreeSet` also collapses multiple outstanding debts on one channel.

**Fix:** Make acknowledgment attribution conservative across close/flush uncertainty. Do not classify the next acknowledgment solely from channel debt or leave an acknowledged, terminated A ahead of B. Add sequences covering a genuinely lost flush acknowledgment, an outstanding experiment acknowledgment during recovery, and repeated flush attempts.

### 2. FakeDevice still implements neither idle-only time checking nor packet-policy dropping

**Where:** `src/device.rs:1844–1882,1896–1905`.

**Problem:** Two concrete sequences disagree with the VERIFIED FPGA behavior:

- After a late SOB packet sets `dropped`, every subsequent non-SOB packet returns immediately at line 1878. It generates no further TimeError and cannot recover on a future acceptable packet. That is sticky burst dropping, not packet policy.
- After starvation drops one timed continuation, the code clears `starved` and `in_burst`. The next non-SOB timed continuation enters the idle branch without checking its timestamp and can play late.

Conversely, a SOB inside active transmission resets/checks time, although the radio checks time only from idle.

**Evidence:** `plan/phase8/reviews/d9b-precheck-sol.md:8–41`, corroborated by UHD v4.10 [`radio_tx_core.v`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v#L337-L409). The shared starvation probe sends only one late continuation, so misses both repeated-packet failures.

**Fix:** Separate radio idle/drop state from DUC burst state. Check each packet encountered from idle, independently of SOB; retain the DUC timeline across radio underflow. Test two late packets followed by an acceptable future packet without SOB.

### 3. Fake flush acknowledgments precede playback, and recovery depends on that fiction

**Where:** `src/device.rs:1927–1930,2015–2017`; `src/provider/tx.rs:713–716`; `plan/maintenance/tools/mutations.json:15030–15038`.

**Problem:** An untimed two-zero SOB/EOB receives acknowledgment timestamp `now`, even when it is appended behind future queued samples. FakeDevice can therefore acknowledge T3 before either flush sample plays. Recovery derives its safe front from that premature timestamp, potentially before the actual flush’s end.

Hardware acknowledges EOB as its last sample goes out; its report is not a flush-start timestamp. The unconditional `ack_tick + 3` therefore also fails to implement one hardware-grounded exclusive-end-plus-one rule. P1185 currently enforces the fake-specific assumption rather than detecting it.

The implementation report explicitly retains this behavior to avoid frozen-clock test stalls. That is not a valid hardware model.

**Fix:** Model acknowledgment at playback completion and define the recovery boundary using verified acknowledgment semantics. Advance/release test clocks around the wait rather than making acknowledgments instantaneous. Replace P1185 with a hardware-shaped boundary test.

## Minor findings

### 4. An acknowledgment returned after the deadline is accepted as timely

**Where:** `src/provider/core.rs:1221–1236`.

Remaining time is checked before `tx_async`, but not after it returns. If the reader resumes after the deadline carrying the final acknowledgment, T3 succeeds without the required timeout rejection.

**Fix:** Recheck the deadline after the blocking read. Preserve a late report without allowing it to establish timely flush completion. Add a gated read returning its acknowledgment after expiration.

### 5. B18 names the wrong test binary

**Where:** `plan/phase7/tools/mutations.json:1277–1284`.

B18 uses `"test_bin": "fake"`, but its killing function exists only at `src/provider/tx.rs:4992`. The integration file contains merely a comment mentioning that name (`tests/fake.rs:3851`).

The current mutation runner rejects zero-test baselines (`plan/maintenance/tools/mutate.py:113–128`), so this row cannot pass as currently written.

**Fix:** Restore `"test_bin": "lib"` and rerun B18 remotely. The reported earlier kill does not validate this current row.

### 6. A failed payload-bearing EOB does not record `tx_close`

**Where:** `src/provider/tx.rs:551–579`; test at `:5126–5137`.

Short/failed EOB sends through `send` record `tx_burst`, then attempt a corrective close. If that corrective close succeeds, no `tx_close` row exists for the original uncertain EOB. The padded-close test checks only the generic send rejection, despite the brief and implementation report promising `tx_close`.

**Fix:** Record the original EOB failure consistently and assert that row for zero/one accepted samples and errors.

### 7. Accepted text still contradicts T2/T3

**Where:** `design/18-uhd-radio.md:212–213,251`.

UR-13 still says arm “sends no buffer”; UR-23 twice prescribes an empty EOB. Both contradict the new implementation and UR-22.

**Fix:** Replace those claims with precise references to T3 and T2. Do not leave contradictory normative copies merely because the changed rules pass the document checker.

### 8. Important planned observations are not exercised

**Where:** `src/provider/core.rs:1481–1517`; `tests/fake.rs:8652–8703`; `tests/common/d9.rs:200–319`.

The opener barrier tests create a synthetic writer that explicitly calls `lock_tx_dsp`; they do not exercise `Control::give`. Removing the production writer’s gate at `src/provider/control.rs:769` would not break these tests.

The differential helpers share sequences, but not verdicts: hardware classifies received tones, while FakeDevice mostly asserts internal DSP logs or acknowledgment timing.

**Fix:** Drive the barrier through the real timed-release path. Share observable verdict calculations for the probes, using explicit tolerances rather than substituting internal state.

## Notes

No out-of-scope R2′, #71 release policy, underflow continuation, MockRadio or Vocabulary implementation was found. KL-12 correctly preserves the pending-FIFO limitation.

The historical tail-test hang is consistent with requesting six RX samples while frozen time covered only five. The current test advances through all six (`src/provider/tx.rs:5853–5854`), but frozen-clock waits still need watchdogs; their device-clock timeouts cannot expire while frozen.

**Not checked:** No edits, builds, tests, mutations or hardware operations were performed. The historical hang was not reproduced; this is a read-only source review.

## Sol, fix1

VERDICT: FAIL

Paths below are relative to `crates/ezsdr-radio-uhd/`.

### Major 1 — Report-triggered close failure permits data before recovery

**Where:** `src/provider/tx.rs:210–226,1098–1100`

**Problem:** `pass()` checks `reopen_needed` only before reading reports. An underflow report can call `abandon()`, whose failed EOB sets that flag. The same pass then calls `step()` and can send a held burst through the uncertain streamer, before reopen/T3.

**Evidence:** The adjacent test at `src/provider/tx.rs:3294–3309` covers a **lost-device** EOB error, which independently blocks `step()`. It misses the non-lost error and short-close cases.

**Fix:** Prevent payload dispatch whenever recovery becomes pending, including during report processing. Test underflow → non-lost failed/short close → held B, asserting streamer replacement and T3 precede B.

### Major 2 — A normal Stop erases required recovery

**Where:** `src/provider/tx.rs:933–940`

**Problem:** `stop_now()` calls `end_open(true)` and then unconditionally clears `reopen_needed`. If Stop’s EOB fails, it immediately erases the recovery requirement. It also erases recovery already pending from an earlier failure.

**Evidence:** A stream-level `TxCmd::Stop` does not end the SampleClock; later bursts are allowed. Thus Stop → failed EOB → later burst sends on the uncertain streamer without reopening. The new Stop test at `:1474–1482` instead sets the terminal Provider-stop flag and does not exercise this path.

**Fix:** Preserve uncertainty across ordinary TX Stop. Reopen before subsequent payload; terminal shutdown need not recover. Test both an EOB failure caused by Stop and Stop arriving while recovery is pending.

### Major 3 — FakeDevice lets host timestamps bypass the surviving DUC timeline

**Where:** `src/device.rs:1833,1851–1858,1889–1893`; test at `:2816–2838`

**Problem:** The fake retains `duc_time` for DSP consumption, but checks radio lateness using the host’s `at`. These are not independent timestamps in the hardware.

**Evidence:** Send two timed samples without EOB, let radio underflow, then send a future-dated continuation without SOB. The fake accepts that continuation and its test requires BurstAck. However, VERIFIED UHD v4.10 [`axi_rate_change.v:396–433`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_rate_change.v#L396-L433) generates outgoing timestamps from the first header and sample count; subsequent host timestamps do not restart it. The idle radio therefore sees the surviving, now-past DUC timestamp and rejects the packet.

The active-SOB branch at `src/device.rs:1874–1881` likewise resets `duc_time` without a preceding DUC EOB.

**Fix:** Feed the radio model the DUC-generated timestamp, and restart DUC time only at its actual burst boundary. Replace the ACK expectation with the hardware-shaped late-packet result; add active-SOB DSP-timeline coverage.

### Minor 4 — Generation replacement leaks TimeError deduplication entries

**Where:** `src/provider/tx.rs:1017–1022`

**Problem:** `sync_streamer()` clears `unacked` but not `late_reported`. Targets discarded with the old generation can never reach the ACK-retirement cleanup at `:346–347`.

**Evidence:** Repeated TimeError → uncertain close → reopen cycles permanently accumulate old target entries.

**Fix:** Clear the generation’s deduplication state with its correlations. Test a late-reported pending target followed by generation replacement.

### Notes

- Normal ticked ACK/TimeError attribution, tickless fallback, post-read flush deadline checking, `tx_close` recording, B18’s binary, and continuation-metadata removal are present.
- P1188–P1197 and B18 were inspected, not executed. The shared probe classifier remains inconclusive on FakeDevice; its assertions are not equivalent RF verdicts.
- **Not checked:** No files changed; no cargo, tests, mutations, or hardware operations run. This is a read-only source review.

## Sol, fix2

VERDICT: FAIL

Paths below are relative to `crates/ezsdr-radio-uhd/` unless stated otherwise.

### Major 1 — Radio rejection incorrectly freezes the upstream DUC

**Where:** `src/device.rs:1853–1858,1909–1931`; `src/device.rs:2788–2816`.

**Problem:** A late packet returns before advancing the DUC sample counter, consuming DSP commands, or processing its EOB. This confuses downstream radio rejection with upstream DUC processing.

**Evidence:** UHD v4.10’s [`axi_rate_change.v:396–433`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_rate_change.v#L396-L433) advances time as output samples are consumed and resets at the DUC burst’s last word. The downstream [`radio_tx_core.v:392–426`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v#L392-L426) consumes discarded packets in `ST_POLICY_WAIT`. Radio rejection therefore does not prevent DUC advancement or its EOB boundary.

Concrete sequence: timed burst → underflow → late two-sample timed continuation with EOB → future timed SOB. The continuation is dropped by the radio, but its EOB ends the DUC burst; the next SOB can establish a new timeline. The fake instead retains the old timeline and rejects the next burst. The changed test at `:2788–2816` explicitly requires this incorrect second rejection. `design/18-uhd-radio.md:366` also ties DUC persistence to radio acceptance.

**Fix:** Process DUC sample-count, DSP and EOB transitions independently of radio acceptance. Add tests for a late continuation without EOB followed by another packet, and a late continuation with EOB followed by a future burst. Correct the existing test and normative wording.

### Major 2 — Failed reopen still clears recovery on a non-terminal path

**Where:** `src/provider/tx.rs:740–755`; `src/provider/core.rs:1285–1296`.

**Problem:** `reopen_streamer()` unconditionally clears `reopen_needed`, including when reopening or its rate check fails without device loss or Provider stop.

**Evidence:** The refusal branch only runs when the active segment has `by: Some(...)`. An initial segment has no initiating command. On that segment, failed close → held B stopped at the new send barrier → non-lost reopen failure clears recovery without halting the clock or removing B. The next pass can enter `send()` and attempt B’s payload without a successful replacement/T3. A failed rate check also closes the replacement streamer before this happens.

The new tests cover successful replacement and terminal cancellation, not failed replacement.

**Fix:** Keep recovery pending after a non-terminal reopen failure, or explicitly halt the stream and settle its bursts—including the initial segment. Test both open failure and post-open validation failure; assert no B payload dispatch before successful recovery.

### Notes

- The shared `send()` barrier fixes the originally reported same-pass dispatch case. Ordinary TX Stop now preserves recovery, and generation replacement clears `late_reported`.
- The D9 probes still assert model-specific timing/report outcomes, **not equivalent RF bench verdicts**: `tests/fake.rs:8676–8678,8690–8692,8708–8710` explicitly requires inconclusive tone classifications. The implementation report overstates that verification.
- P985/P986/P1181 and P1198–P1202 each match their source exactly once. Their killing tests were inspected, not executed.
- The failure-mechanism record names the adopted findings, but its claim that they are fully pinned is premature given the sequences above.

**Not checked:** Read-only review; no files changed and no cargo, tests, mutation runs or hardware operations executed.

## Sol, fix3

VERDICT: FAIL

Paths below are relative to `crates/ezsdr-radio-uhd/` unless stated otherwise.

### Major 1 — Failed reopen leaves the burst already stopped at the send barrier uncancelled

**Where:** `src/provider/tx.rs:753–759,771–795`; test at `:1589–1610`.

**Problem:** Failed-reopen cleanup drains `self.held`, but never removes or settles `self.open`. The recovery barrier can already have moved the next burst B into `open`.

**Evidence:** `ur_22_underflow_close_failure_reopens_before_the_next_payload` explicitly asserts this reachable state at `:1568`. If that sequence’s replacement open or post-open check fails, `halt_failed_reopen()` clears the clock and tracker but retains B and its reservation. With no later segment, B remains stranded until Stop/shutdown, without the required failed-reopen cancellation record. The new failure test misses this because it inserts B into `held` and immediately requests recovery.

**Fix:** Settle `open` without another device send before discarding the clock/tracker: remove its reservation, record its cancellation, and finish any existing burst record. Extend the barrier sequence with both reopen failure kinds; assert `open` is empty, B’s reservation is gone, cancellation is recorded, and no B payload was sent.

### Major 2 — An untimed SOB incorrectly preserves the DUC’s timed-command consumption

**Where:** `src/device.rs:647–655`; existing test at `:3230–3262`.

**Problem:** Clearing `has_time` requires `!sob`. An untimed start-of-burst packet inside an unfinished DUC burst therefore leaves timing enabled, and subsequent timed packets keep consuming DSP commands incorrectly.

**Evidence:** UHD v4.10’s [`axi_rate_change.v:396–433`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_rate_change.v#L396-L433) sets `has_time_clear` whenever an input packet lacks time; there is no SOB exception. Later timed headers do not restore it before EOB.

Concrete sequence: timed packet without EOB → untimed SOB without EOB → timed continuation containing a pending DSP command’s logical instant. The fake consumes that command in the continuation; the hardware’s cleared `has_time` prevents consumption. UR-33’s wording at `design/18-uhd-radio.md:365–366` likewise says a later send without time ends timed consumption. The existing test covers only `sob=false`.

**Fix:** Remove the SOB exception from the untimed-header transition, preserving the hardware’s packet-boundary clearing behavior. Add the sequence above and assert the command remains pending until a fresh timed burst.

### Notes

- The original radio-rejection/DUC coupling is fixed: rejected packets now advance DUC flow and process EOB independently.
- The send barrier, ordinary Stop preserving recovery, and generation-scoped report deduplication remain present.
- P1203–P1207 and retargeted P212/P985/P986/P1185/P1194/B30 each match their source exactly once. Their assertions were inspected, not executed.
- The implementation report and failure-mechanism record now correctly distinguish model-level probe checks from RF verdicts.

**Not checked:** Read-only review; no files changed, builds, tests, mutation runs, or hardware operations performed.

## Sol, fix4

VERDICT: PASS

No major or minor findings.

- **Note — M1 fixed:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:771–801` settles `open` before clearing the clock/tracker: releases its reservation, records cancellation, finishes any burst record, and sends nothing. Cleanup is unconditional across segments. The barrier test at `:1553–1610` covers both replacement-open refusal and post-open rate-check failure.

- **Note — M2 fixed:** `crates/ezsdr-radio-uhd/src/device.rs:652–655` clears timing on every untimed packet, including SOB, consistent with UHD v4.10 `axi_rate_change.v`’s `has_time_clear` transition. The test at `:3230–3250` checks that timed continuation cannot restore consumption before a fresh burst.

- **Note — Revised mutation tests observe their guarantees:** `crates/ezsdr-radio-uhd/tests/fake.rs:4173–4202` requires the experiment close’s exact ACK tick, excluding T3’s earlier ACK; `:3839–3855` requires the one-sample burst’s timed SOB/EOB. P1181/P1182’s tests at `src/device.rs:3255–3288` use manual-clock participants rather than scheduling-sensitive timing windows. All inspected mutation anchors match exactly once; P1158 remains explicitly equivalent.

- **Note — Earlier fixes remain intact:** The payload recovery barrier, Stop preserving recovery, generation-scoped report cleanup, tick attribution, T1–T3 ordering, and separate DUC/radio transitions show no regression in the inspected paths.

**Not checked:** No files changed; no builds, tests, mutation runs, or hardware operations performed. The implementation report’s execution results were not independently rerun.

## Sol, test plan

VERDICT: REVISE

The plan covers the main successful paths, but its TimeError attribution assumes the wrong timestamp meaning. T3 also needs explicit tests for concurrent command release and failed flushes before implementation.

## Independent failure scenarios

Derived from the existing implementation and UHD source before comparing with the plan:

- A two-sample closing send accepts only one sample: EOB is not delivered, yet the next burst is treated as safe.
- A late packet from burst A produces an error after A’s scheduled end, possibly during B’s scheduled range.
- An owner reopens TX while uhd-control releases a timed frequency command: the command reaches the stale DUC before flush acknowledgment.
- A flush acknowledges one channel but not another, or continually returns unrelated reports: initialization must not complete early or wait indefinitely.
- A flush times out; its acknowledgment arrives later and must not retire an experiment burst’s correlation.
- A timed DSP value survives close/reopen: T3 clears effective stale-header state but leaves that value queued.
- Raw P5 A/C, P6 starvation, P7 and D70 must retain their failing/control behavior independently of the Module’s fixes.

## Blocker decision

### Keep untimed configuration in `prepare`

**Confirm the orchestrator’s narrower ordering recommendation, with qualifications.** Do not move TX configuration out of `prepare` merely to precede it with T3.

UHD v4.10 routes untimed writes directly to the untimed settings bus ([`axi_tag_time.v:91–97`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_tag_time.v#L91-L97)). The DDS accepts that phase increment without comparing its timestamp ([`dds_timed.v:348–366`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/dds_timed.v#L348-L366)). A stale header alone therefore does not defer an untimed frequency write.

Revise T3 to require **completion of the flush acknowledgment window before the next timed TX DSP issuance to that chain**, rather than before every untimed configure. The successful flush must also precede experiment data on that data path.

This is **not a clean-FIFO guarantee**. Untimed flush samples do not dequeue pending timestamps: dequeue requires `has_time` ([`axi_tag_time.v:206–211`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/axi_tag_time.v#L206-L211)). A surviving timed value can still override configuration during the next timed burst. That remains the explicitly deferred #71/R2′ problem. A missing acknowledgment is likewise an acknowledged degraded path, not proof that initialization completed.

### The opener reads the acknowledgment

The proposed ownership direction is sound:

- **`arm`:** reads before worker threads start.
- **uhd-control from-none:** retains `FromNone`, waits for any existing `reading` operation to end, then sends and reads the flush itself.
- **uhd-tx reopen:** reads inside its existing `Owner` claim before returning it.

Do not use ordinary `begin_read` inside those claims: it requires `Free` (`crates/ezsdr-radio-uhd/src/provider/core.rs:374–380`). Do not release the claim to obtain it. Keep `streams` unlocked during device calls.

Normal polling is excluded by those claims. **Shutdown is an explicit exception**, however: `begin_shutdown_read` ignores the holder (`core.rs:383–386`). Test that interaction rather than claiming the opener’s hold excludes every reader.

## Findings

Paths below use:
- `plan` = `/home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/d9b1a-test-plan.md`
- `brief` = the adjacent `impl-d9b1a-brief.md`.

### 1. P1 — TimeError ticks do not identify the rejected packet’s scheduled range

**Evidence:** `plan:29,59`; `brief:20`; `crates/ezsdr-radio-uhd/src/provider/tx.rs:974–990`.

The proposed test injects errors whose ticks already fall inside A’s range. Hardware timestamps the error with **current radio time**, not the rejected packet’s requested timestamp: [`radio_tx_core.v:450–460`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v#L450-L460). UHD preserves that timestamp when creating TIME_ERROR.

For example, A’s late continuation can be rejected after A’s nominal end, while B’s scheduled range is current. Range matching can blame B; the planned fixture cannot expose this.

**Fix:** Amend the attribution requirement before implementation. Add errors observed after A’s range, during B’s range, after abandonment, and without a tick. Never consume B’s pending target merely because an error tick lies in B’s range. Where the report cannot uniquely identify a burst, preserve the raw report and leave the target unclaimed rather than inventing attribution.

### 2. P1 — T3’s timed-command barrier is not tested

**Evidence:** `plan:27,65–67`; `crates/ezsdr-radio-uhd/src/provider/control.rs:763–788`; `crates/ezsdr-radio-uhd/src/provider/core.rs:1270–1275`.

The plan checks sequential open/flush/configure order. But `Control::give` can issue timed TX settings independently; its existing serialization against configuration does not itself establish a flush barrier.

An owner can be waiting for T3’s acknowledgment while uhd-control issues a timed DSP value against the stale header.

**Fix:** For owner reopen, gate acknowledgment delivery and attempt a concurrent timed-frequency release. Assert no timed TX DSP write occurs until acknowledgment or the recorded timeout. Cover the release boundary for all three open routes, without holding `streams` over the wait.

### 3. P2 — Flush failure and acknowledgment identity are underspecified

**Evidence:** `plan:27–28`; `crates/ezsdr-radio-uhd/src/provider/tx.rs:918–943,996–1011`.

“No acknowledgment” does not cover short/failed flush sends, repeated immediately failing reads, a stream of unrelated reports, partial multi-channel acknowledgment, device loss, or a late acknowledgment after timeout.

Reusing the shutdown drain loop mechanically is unsafe: its inner report loop does not check the outer deadline while reports keep arriving.

**Fix:** Specify a total deadline with remaining-time read bounds and progress on immediate errors. Add deterministic tests for:
- zero/one accepted flush sample and send error;
- repeated read errors/unrelated reports;
- acknowledgment from every relevant channel;
- loss and Stop during the wait;
- late flush acknowledgment not retiring experiment correlations.

Keep genuine device loss distinct from an ordinary missing acknowledgment.

### 4. P2 — T2 tests fault the payload, not the closing send

**Evidence:** `plan:25`; `crates/ezsdr-radio-uhd/src/provider/tx.rs:627–636`.

The planned ShortSend/FailSend cases verify that a two-zero close is **attempted** after payload failure. They do not test that the closing send itself accepts zero or one sample, or fails.

Two samples requested is not two samples successfully transferred. Advancing `front` cannot establish that the DUC burst closed.

**Fix:** Fault the EOB-bearing send itself, including a padded one-sample payload close. Specify bounded recovery or explicit uncertain-state rejection; assert no subsequent timed burst is reported as safely initialized solely from the attempted close.

### 5. P2 — Pending FIFO state needs an explicit negative test

**Evidence:** `plan:27,42–43`; `plan/phase8/39-dsp-timed-commands.md:109–115`.

The successful stale-header tests contain no surviving timed command. They cannot distinguish “header cleared” from “device state clean.”

**Fix:** Add a same-process reopen case with a pending timed DSP value. Assert T3 does **not** drain it and document that its later override remains deferred to D9b-1b. Do not broaden KL-12’s closure or T3’s guarantee to cover that case.

No P3 findings.

**Not checked:** No edits, builds, tests or hardware operations performed. Review used HEAD `fe27833`, the supplied plans, bench records and UHD v4.10 source; deployed-image equivalence remains unverified.
