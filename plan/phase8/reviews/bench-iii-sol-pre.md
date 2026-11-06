Failure scenarios considered first: false PASS despite lost samples or faults; noise mistaken for filter fill; a wrong timing reference; unsafe continuation after failed cleanup; and evidence overwritten between boards.

1. **FIX FIRST — TX200 can transmit on slot B.** `crates/ezsdr-radio-uhd/tests/hardware.rs:2639–2661` adds grouped/split trials whenever the layout has two TX channels; #2 has CBX in B (`plan/phase8/bench.md:401–404`).
   **Brief change:** explicitly constrain TX200 and LOAD200 to slot A before constructing stages. If using `EZSDR_MEASUREMENT_SELECTOR`, pass it through `hw()` and assert the resulting channel set.

2. **FIX FIRST — The postponed TX200 path lacks T4’s safety stop.** After `Err` or `left_open`, TX200 still proceeds to another trial (`crates/ezsdr-radio-uhd/tests/hardware.rs:2672–2689`).
   **Brief change:** apply T4’s stop-on-error/open-streamer rule to TX200 too. Never reopen or continue on that handle; require owner intervention/power-cycle as appropriate.

3. **FIX FIRST — #78’s blanket zero-TIME_ERROR rule contradicts intentional controls.** `hw_b8_burst_one_sample_after_a_burst` explicitly requires one TIME_ERROR for end+1 (`crates/ezsdr-radio-uhd/tests/hardware.rs:1471–1478`); preemption deliberately probes inside the in-flight window (`:735–750`).
   **Brief change:** distinguish expected policy events from unexpected faults, by case. Require zero unexpected TIME_ERROR and zero underflow; do not remove the negative controls to satisfy the closing rule.

4. **FIX FIRST — Checking expectations against UR-21/22 does not establish fault coverage.** P70 counts faults only in the one-sample trial (`crates/ezsdr-radio-uhd/tests/hardware.rs:2881–2884`); P72’s verdict omits them (`:2840–2857`); B0 discards its Manifest (`:1943`); preemption only prints.
   **Brief change:** require a per-case evidence/check inventory, not merely “matches.” P70 must check its control too before #70 closes; every #78 case must expose and judge the required faults. INCONCLUSIVE/test-process success is not closure evidence.

5. **SHOULD — T3’s different-rate calibration hypothesis is already disproved by the code.** The control is sent at 2 MS/s and `delay2` supplies B’s reference (`crates/ezsdr-radio-uhd/tests/hardware.rs:3467–3494`). The logged delta is **+3,200 ticks**, not an uncalibrated +6,600.
   **Brief change:** state this as VERIFIED. Investigate the exact rejected 25-sample window and missing control-1 coverage; `first_ms_tone` rejects missing/incoherent covering windows (`tests/common/tone.rs:159–174`). RX is not restarted between the control and B, so restart-fill causation remains INFERRED. Preserve the existing onset tolerance and INCONCLUSIVE result.

6. **FIX FIRST — T2 can measure noise or the wrong rate and call it fill.** Its PASS conditions do not require signal evidence or matching rate read-back; a disconnected loop can produce threshold crossings against its own noise median. The reused rate probe does check rate and tone (`crates/ezsdr-radio-uhd/tests/common/n47.rs:32–65`).
   **Brief change:** require matching `Applied.rate`, continuous coverage of the complete analysis span, and a usable steady 100 kHz tone before reporting fill. No signal is INCONCLUSIVE; rate mismatch must not PASS. Add corresponding pure-function tests.

7. **SHOULD — T2 does not receive raw sc16 integers.** UHD is configured as CPU `fc32`, wire `sc16` (`crates/ezsdr-radio-uhd/src/uhd.rs:513–516,968–972`), returning allocated float IQ (`:1002–1036`).
   **Brief change:** describe exact zeros in converted fc32, without claiming integer-wire observations. At 100/200 MS/s bound retained blocks per trial and check continuity explicitly; `ProbeDevice` retains/clones samples and `read` requests only 2,000 samples (`tests/common/probe_device.rs:131–138,400–401`; `tests/hardware.rs:389–399`).

8. **FIX FIRST — LOAD200 can PASS while the sink loses samples.** Counting occurs before publication (`crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:427–430,845–853`), but the sink link drops oldest at capacity 64 (`:725–726`); stage classification never checks link drops (`:903–914`).
   **Brief change:** report and reject nonzero Manifest link drops in the shared runner/judgment, with an injected-drop rehearsal. Otherwise explicitly limit the claim to device-to-host reception, not loss-free Module delivery.

9. **SHOULD — NIC evidence is absent by default.** `MeasurementPlan::default()` leaves `nics` empty (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:650`); the runner reads only byte counters (`measurement/multichannel.rs:670–685`).
   **Brief change:** populate/pass the board’s NIC and record error/drop/missed-packet counter deltas alongside bytes. Record that 200 MS/s is 800 MB/s **per direction** on full-duplex 10 GbE, while fc32 host buffers require 1.6 GB/s per direction before further copies.

10. **SHOULD — T1’s requested comment correction reverses the actual clock meaning.** `tx_at` returns a SampleClock instant (`crates/ezsdr-radio-uhd/tests/common/mod.rs:909–912`); B adds 10,000 ticks **on that clock** (`tests/hardware.rs:1493–1494`).
    **Brief change:** say “10,000 sample-clock ticks = 10,000 samples at 1 MS/s = 10 ms,” not 50 samples from interpreting them as master ticks.

11. **SHOULD — Copying `hw()` overwrites the first board’s evidence.** Its filename contains only the test name (`plan/phase8/bench.md:413–420`).
    **Brief change:** include board/address and trial identity in log paths. Keep the 30-versus-40 dB discrepancy explicitly owner-resolved before TX (`bench-results.md:279` versus `bench.md:400`).

12. **SHOULD — The mutation instructions conflict with repository policy.** Brief line 47 requires manual mutant probes while prohibiting the sole authorized runner; `AGENTS.md:165` requires `remote_mutate.py`.
    **Brief change:** leave row execution to the orchestrator, or authorize the permitted remote runner; retain unmutated unit tests and anchor checks locally.

Checked UHD source in image `ezsdr-v4-dev:uhd4.10` at `/root/tmp/uhd-4.10.0.0` (`/usr/local/src` was empty). The sink creates no sample file without a request (`crates/ezsdr-sink-capture/src/lib.rs:377–378,413–416,204–206`); its sample queue is bounded. No hardware tests, builds, formatting or edits performed; actual throughput and RF conditions remain unverified.

**FIX FIRST — 6 findings.**
