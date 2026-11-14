Before reading the brief’s designs, my failure scenarios are:

- **C2:** High-rate chunks all arrive after their own timestamps, making silence non-discriminating; or missing reports hide the packet-overhead measurement.
- **C3:** The DUC enlarges its output packets after the 32-sample head, so a whole-burst verdict alone does not measure small-packet overhead.
- **C4:** B reaches the device only after the drain finishes, or host feeding pauses extend A’s drain. Either can produce a boundary unrelated to the fake’s pad. The first error tick dates error detection, not raw packet arrival.
- **C5:** Independent channel checks produce a genuine split near the deadline. Coarse host-timed offsets can miss that region; both channels silent does not establish atomic behavior.
- **C6:** The head never plays, or the tail does not actually arrive late. Missing reports under load must not count as proof of report order.

These follow from `mt3.rs:140–153`, `device.rs:735–756`, and the FPGA state machines. I found UHD 4.10 host and FPGA sources inside the local `ezsdr-v4-dev:uhd4.10` image at `/root/tmp/uhd-4.10.0.0`; the requested file-name search found none under host `~/`, `/usr/local/src`, or `/opt`.
## Verdict: FAIL

The brief needs measurement-design changes before implementation, chiefly C4’s validity checks, C5’s interpretation, and C6’s shape and evidence requirements.

References: **B** = the reviewed `brief.md`; **U** = `/root/tmp/uhd-4.10.0.0` inside the local `ezsdr-v4-dev:uhd4.10` image.

### Major

1. **C4 can blame the fake’s pad for host starvation.**
   - **VERIFIED:** B:27 checks only when B was handed. The FPGA waits when a draining packet has no data; it cannot skip missing samples (`U/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v:399–426`).
   - **INFERRED counterexample:** A’s feeder stalls midway, extending its drain beyond `t0 + S·per + pad`; B is handed before the specified deadline but is dropped. The sweep reports **FAIL**, although the extra delay came from the host, not insufficient device-overhead padding.
   - **Fix:** Record every A send’s sample range and completion tick. Require a justified delivery margin ahead of its drain-consumption deadline; otherwise **INCONCLUSIVE**. Likewise, “B handed 2 ms early” is only an **INFERRED transport margin**, not proof B was queued. Record and qualify that assumption. Exclude Underflow/sequence-error trials from the pad verdict: a partially transmitted B must not be classified as wholly dropped.

2. **C4’s reference needs a clock-offset definition, and its sweep does not resolve the boundary.**
   - **VERIFIED:** `ERR_TX_LATE_DATA` is generated in `ST_TIME_CHECK`; the FIFO captures **`radio_time_d1`**, not the packet timestamp or an arrival timestamp (`radio_tx_core.v:280–284,337–355,450–459`). UHD forwards that timestamp (`U/host/lib/rfnoc/radio_control_impl.cpp:1302–1307`).
   - **INFERRED:** `t0` is a useful near-drain-start reference, but calling it “A’s arrival” and treating it as the fake’s exact drain origin omits pipeline-clock alignment.
   - **Fix:** Define the relationship and uncertainty between `t0`, first discarded sample, and the fake’s origin. Compare corresponding boundaries with that uncertainty. Keep `t0 + S·per + δ` as a measurement coordinate, not an asserted exact endpoint.
   - **VERIFIED:** B:29 samples the likely device transition only every 5–10 µs; the underlying packet overhead is two **master clocks**, and the DUC has a post-EOB wait (`radio_tx_core.v:299–355`; `U/fpga/usrp3/lib/rfnoc/axi_rate_change.v:275–282`).
   - **Fix:** Use the grid to bracket the transition, then refine in integer ticks with ≥10 trials per refined cell. Report the bracket and mixed outcomes, not merely “smallest δ with every trial played.”
   - **VERIFIED:** The verdict’s **direction is correct**: device dropping where the corresponding fake accepts violates UR-33 (`src/device.rs:735–740,831–838`).

3. **C5 treats the phenomenon it seeks as an unexpected failure, while absence can pass without useful marginal evidence.**
   - **VERIFIED:** Separate DUC chains exist (`U/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_duc/rfnoc_block_duc.v:242–244,310–323`). Fable explicitly predicts a possible split near the deadline (`fable-review.md:182–185`).
   - **INFERRED counterexample:** All four host-relative offsets arrive clearly late because of control-call latency. Both channels drop every time; B:36 nevertheless permits **PASS**, without probing channel skew. Conversely, a genuine split is the expected independent-channel possibility, not a refutation of the per-channel late-drop rule.
   - **Fix:** Separate **measurement validity** from **finding a split**. Report `split observed` as positive evidence requiring contract review; report no split only for the tested conditions. Require decided outcomes and an observed early/late transition before declaring marginal coverage sufficient. Sweep/refine around that measured transition; do not assume `{0,10,20,50} µs` brackets it.

4. **C6 does not send the Module’s closing shape.**
   - **VERIFIED:** The Module closes with **two samples: held tail plus zero**, timed at the tail’s instant, SOB=false, EOB=true—not one sample (`src/provider/tx.rs:865–886`). The existing raw helper documents the same shape (`tests/common/raw.rs:183–185`).
   - **INFERRED wrong outcome:** A one-sample experiment can pass while leaving the actual Module close unmeasured.
   - **Fix:** Send the two-sample close at `first + (S−1)·per`, delayed ≥1 ms. The deterministic hold is sufficient for the device-order question; adding host load is unnecessary.

5. **C6’s PASS can lack TimeError evidence or test an earlier starvation.**
   - **VERIFIED:** B:40 permits PASS with an Underflow and “every TimeError after it”; that condition is vacuously true with **zero TimeErrors**. It also does not establish where the head starved.
   - **INFERRED counterexample:** The head underflows early, the closing report is lost, and the trial passes without demonstrating the held-tail sequence.
   - **Fix:** Require **both** Underflow and TimeError, on the same channel, with attributable ticks and the required order. Missing either is **INCONCLUSIVE**. Establish that the head played through the intended final pre-tail sample, or provide equivalent discriminating timing evidence.
   - **VERIFIED:** R3 predicts Underflow followed by late-data errors after starvation; R4 can discard the remainder of an interrupted packet first (`radio_tx_core.v:359–370,399–410`). **INFERRED:** those are C6’s expected results only after its head/tail preconditions hold.

### Minor

6. **C2/C3’s overhead observations are circular or truncated.**
   - **VERIFIED:** B:19 derives packet length from the same gaps used to measure overhead. First-to-last report time also omits the last packet’s drain (B:24). The FPGA can drop reports when its error FIFO fills (`radio_tx_core.v:436–458`).
   - **Fix:** Log full report ticks and independently known packetization assumptions. Identify skipped-report gaps; do not infer “+2” from arbitrary adjacent reports. Label first-to-last duration as a partial span unless the last packet’s contribution is established. Keep these observations outside the late-drop verdict.

### Checks that do not require redesign

- **VERIFIED:** RX 10 MS/s with TX 100/200 MS/s is architecturally sound: UHD sets the DDC output and DUC input rates separately (`U/host/lib/usrp/multi_usrp_rfnoc.cpp:1314–1332,1999–2017`). Read back both actual rates.
- **INFERRED:** Twenty milliseconds is sufficient at an achieved 1.5× delivery rate: roughly 13.3 ms of sending leaves several milliseconds of headroom. It is not a throughput guarantee; retain the measured last-chunk margin.
- **VERIFIED:** RF conditions and per-command owner confirmation are specified appropriately (B:13,34,43). Reusing `two_channel_gate` correctly rejects the current CBX slot-B configuration (`tests/common/bench1115.rs:1408–1430`).

**Not checked:** No hardware, builds, formatters, or mutation tools ran; no files changed. Historical bench measurements quoted by Fable were not independently revalidated against raw records.
