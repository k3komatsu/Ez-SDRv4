# 段階 D9a：Sol 6.1 のコードレビュー

計画のレビューは [d9a-sol-plan.md](d9a-sol-plan.md)．

## コードレビュー
VERDICT: FAIL

## Independent failure scenarios

- Stopped or future RX chains accumulate commands, or lose deferred values at restart.
- Release crosses configure’s snapshot/publication boundary and loses a value.
- A move’s destination applies an untimed value while the source still produces samples.
- Origin/cut boundaries, refusal, loss or shutdown change delivery or records.
- Fake DSP queues consume without sample flow, use host rate, or change past samples.
- TX continuation timing, overflow or streamer replacement hides F3′/S2.
- Probes report an apparent hardware result without establishing its prerequisites.

## Findings

### 1. Major — Subdevice-map replacement leaves imaginary RX flow running

**Location:** `crates/ezsdr-radio-uhd/src/device.rs:903–929`, `:1071–1090`

`set_subdev_spec()` clears RX streamers but never closes their new DSP flow intervals. `drain()` subsequently treats those intervals as active indefinitely.

**Concrete sequence:** start RX; queue a frequency for a future instant; replace the RX subdevice map before that instant; advance time without opening or starting another streamer. The entry is consumed despite there being no RX streamer. Repeated writes can therefore avoid the idle-chain failure that this change is intended to expose.

**Evidence:** `st.rx.clear()` at line 1081 bypasses `end_rx_flow()`, unlike `rx_close`, replacement through `rx_open`, and `close_streams`. The added drain uses `chain.flows`, not the streamer map.

**Fix:** end each affected RX flow under the old channel map before clearing it. Add a controlled-clock regression proving that map replacement stops consumption until a real restart. Changing the old frequency-across-map tests to gains did not cover this new lifecycle obligation.

### 2. Major — P5 can report a false early shift and an invalid negative result

**Location:** `crates/ezsdr-radio-uhd/tests/hardware.rs:1178–1207`

Two observations undermine the F7 probe:

- `moves` searches from **four milliseconds before transmission starts**. Those RX windows contain noise, whose estimated phase frequency can differ from 100 kHz. The probe can therefore print an early “tone leaves 100 kHz” even when the actual tone changes exactly at `e`.
- The probe never establishes that the previous timed header’s timestamp is at least the new command’s instant. That inequality is required by `axi_tag_time.v`’s tag comparison. Both openings schedule approximately 150 ms ahead, but the priming burst ends at `start + 10 ms` and the new command is due at `start + 20 ms`. With similar setup durations, the stale timestamp is too small to trigger F7. A negative result would not test the hypothesis.

**Fix:** detect movement only in signal-bearing windows after the burst begins. Record and deliberately establish the old-timestamp/new-instant inequality; otherwise print **INCONCLUSIVE**. Preserve the bounded entry count and RF conditions.

### 3. Major — The concurrency tests do not establish the promised publication ordering

**Location:** `crates/ezsdr-radio-uhd/src/provider/control.rs:6246–6329`; `plan/maintenance/tools/mutations.json:13147–13155`

M3 pauses at `Core::owner_configure`’s `"configured"` probe, **after `ColdConfig::configure()` has returned**. Moving `configured.store(true, …)` outside the `updates` lock but keeping it before return would still pass M3. P974 deletes the store; it does not test that dangerous relocation.

M4a/M4b also treat a 300 ms absence of completion as proof of lock contention. Their worker sends `"started"` **before** calling booking/release. Descheduling immediately afterward can make an unlocked mutant appear to wait; after the owner resumes, the remaining ordering assertions can also pass.

**Fix:** add a deterministic observation at the publication/lock boundary and a mutation that unlocks before publishing. Replace the short negative-timeout contention claims with a handshake that establishes the relevant operation reached the boundary. The implementation currently publishes under the lock correctly, but the accepted race regression is not secured.

### 4. Major — The new TX specification overstates F3′

**Location:** `design/18-uhd-radio.md:265`; `plan/phase8/33-channels-and-streams.md:175`

Both say that a frequency due inside a burst waits until the next burst. This is false when `e` falls within the first timed send while `has_time` still holds.

**Evidence:** the new `ur_33_a_timed_duc_value_is_taken_only_while_has_time_holds` explicitly tests this counterexample at `device.rs:2905–2913`: a command due one millisecond into the first timed send is consumed there.

**Fix:** qualify F3′ as applying to instants after the burst has lost its timestamp, rather than every instant inside the burst. Keep the accepted rule and decision record consistent.

## Other review results

- R1’s cover predicate, destination-specific configured mark, initial-segment handling and shared error/record paths otherwise match the accepted design on static inspection.
- P965–P989 and J326/P214/P215/P219/P409 each match exactly once. Their substitutions target meaningful decisions, subject to finding 3.
- Every `ur_…` test cited by spec 18 exists.
- Omissions 2c, 3d, 12b, 14 and 15 are not independently blocking: their relevant configuration, error and settlement mechanisms remain shared. They do not compensate for the missing publication-order regression.
- No R4/R2/F7 production fix, MockRadio, vocabulary or profile change was found. Probe transmissions retain the existing RF setup; attenuator confirmation remains a prerequisite.

**Not checked:** no files changed, builds, tests, mutations or hardware operations run. FIFO capacity, persistence across streamer close and deployed-FPGA behavior remain unverified.

## 見直し 1
VERDICT: FAIL

### 1. Major — P5’s new signal threshold rejects the bench’s actual tone

**Location:** `crates/ezsdr-radio-uhd/tests/hardware.rs:1212–1228`

The new window filter requires mean received magnitude **> 0.2**. Both `raw_rx` and `raw_tx` use 0 dB gain, and the transmitted tone has amplitude 0.4. The measured loop’s tone amplitude gain is **0.0206** (`plan/phase7/bench-results.md:515`), giving an expected received magnitude around **0.0082**—roughly 24 times below the threshold.

Thus a valid capture can produce zero accepted windows and `moves: None`, even when the tone shifts early. The output can still say “stale-header prerequisite established”: INCONCLUSIVE currently covers only the timestamp inequality, not an unusable signal observation. The original false-negative problem remains.

**Fix:** identify signal using a measured noise-relative threshold or tone coherence, restrict windows to the transmitted burst, and print **INCONCLUSIVE** when usable signal windows are absent. Add a small synthetic check with attenuated 100→150 kHz tones and noise, covering both early and on-time shifts.

### Other checks

- **Previous major 1 resolved:** subdevice replacement ends each RX flow under the old map before clearing streamers. The regression checks that consumption stops and resumes at restart. Nearby `rx_close`, replacement through `rx_open`, and `close_streams` remain consistent.
- **Previous major 2 partially resolved:** P5 deliberately increases the old timestamp, checks `header ≥ e`, and excludes pre-burst windows. The signal-threshold defect above remains.
- **Previous major 3 resolved on static inspection:** `mark_publishing` observes the lock immediately before publication; P991 unlocks before that observation. M4a/M4b now observe entry into the projection-lock operation rather than inferring contention from 300 ms of silence. The wall-clock bounds serve as watchdogs, not ordering assertions. ClockMutex instrumentation is test-only; the production probe is a no-op.
- **Previous major 4 resolved:** spec 18 and design 33 distinguish instants within the first timed send from instants after timestamp loss.
- **mi-1–mi-5:** the wording contradiction, configure ordering, gain substitution, missing lock-removal mutation, and untimed-refusal coverage are addressed. The new refusal test does not itself assert projection retention; that remains covered through the shared refusal-path tests.
- P226 and P990–P993 each match their target source exactly once.

**Not checked:** no edits, builds, tests, mutations or hardware operations performed. MA-1 was excluded as requested; actual P5 hardware behavior remains unverified.

## 見直し 2
VERDICT: FAIL

### 1. Major — Missing RX samples can turn an on-time shift into a late verdict

**Location:** `crates/ezsdr-radio-uhd/tests/common/tone.rs:64–75`

`usable()` counts only windows present in the capture, not coverage of the expected interval (`tone.rs:33–34`). The three-window rule verifies continuity within the detected run, but not between it and the preceding observation.

Concrete case: the tone shifts correctly at **e = burst start + 20 ms**, but RX samples from **19–40 ms** are missing. With valid pre-burst noise and coherent tone elsewhere:

- Both sides pass the signal-window checks.
- Three shifted windows beginning at 40 ms satisfy the move rule.
- The verdict reports movement **20 ms after e**, although the transition was not observed.

This input is reachable: `read_on` preserves packet timestamps across missing samples and records receive errors separately (`crates/ezsdr-radio-uhd/tests/hardware.rs:375–382`). P5/P6/P7 pass the sample vector to the classifier without rejecting such gaps.

**Fix:** detect timestamp discontinuities. When missing coverage prevents locating the first departure, report **INCONCLUSIVE** or a transition interval—not a definite movement time. Add the gapped, on-time-shift case to the synthetic test.

### Other checks

- The absolute-magnitude defect is fixed: normalized coherence does not reject a clean tone merely because its amplitude is approximately 0.008.
- P5/P6/P7 consistently use 1 Msps captures and 25-sample windows. Their classifier arguments and P5’s timestamp-prerequisite guard are consistent.
- The synthetic test genuinely supplies attenuated tones plus noise and checks early, on-time, unchanged, noise-only and missing-floor cases (`crates/ezsdr-radio-uhd/tests/fake.rs:8283–8319`). It does not cover capture gaps.

**Not checked:** no files changed, builds, tests, mutations or hardware operations performed. Real bench noise remains unverified.

## 見直し 3

修正の回 3（受信の sample の欠け）は狭い直しなので，orchestrator が読んで確かめた（[d9a-opus.md](d9a-opus.md) §2）．
