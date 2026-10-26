# 段階 3c-1：Sol 6.1 のレビュー

## コードのレビュー（`sol-3c1.log`）

VERDICT: FAIL

### Failure scenarios checked

- A timing candidate never reaches the Provider, or receives another trial’s evidence.
- An early TX configuration is mistaken for proof of an on-time transmitted burst.
- Overflow loss measures the imposed reader stall and host buffering rather than device recovery.
- Healthy TX is rejected because its SampleClock never ends, includes idle time, or excludes already-sent samples.
- Synthetic tests pass while the real measurement cannot satisfy their assumptions.
- FakeDevice observations become measured values, or record metadata misidentifies the selected boundary.

### Findings

#### 1. Major — TX throughput always lacks its required end timestamp

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1587–1593,1682–1684`

`expected_tx_samples()` delegates to a function requiring `clock.ended_at`. But this measurement never cold-changes or disables TX: it only calls `run.finish()`.

An ordinary TX clock deliberately survives Stop. `timeline.rs:934–940` treats TX End as ending bursts, not cutting the segment; `provider/core.rs:703–717` ends a TX clock only when its segment has a cut. `provider/tx.rs:939–955` closes the burst without ending its clock. The coordinator copies the clock records into the Manifest unchanged (`coordinator/ending.rs:234`).

Consequently, every throughput stage has `expected_samples: null` and becomes `Incomplete`, even on healthy hardware. The rehearsal accepts that classification, while the pure test supplies an ended synthetic clock the measurement cannot produce.

**Fix:** Derive the transmission interval from evidence this Run actually produces, rather than requiring the lifetime of an unended TX clock. Add coverage showing that an ordinary completed repeat Run supplies usable evidence; merely allowing arbitrary classifications does not catch this defect.

#### 2. Major — TX sample accounting compares different intervals

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1620–1621,1629–1630,1645–1673`

Even after fixing finding 1, clock-origin-to-stop is not the interval represented by `tx_samples`.

- **Initial deficit:** TX’s initial clock starts at arm (`provider/mod.rs:588–593`), but the repeat is scheduled at T0 (`tests/common/mod.rs:211–218`), with a configured two-second startup lead (`:163`). Counting from clock origin therefore includes substantial time when no samples were scheduled.
- **Final excess:** TX sends ahead by `tx_in_flight_ns` (`provider/tx.rs:300–302,328–332`). `tx_samples` counts those handed-over samples (`:510`), including samples beyond a subsequently booked stop instant. At 10 MS/s, a 10 ms window can represent roughly 100,000 samples—not one packet.
- **Wrong tolerance:** RX `max_num_samps` is not evidence of the TX packet size or either interval mismatch.

Thus a healthy transmitter can fail the capacity sweep because of the measurement’s bookkeeping. The synthetic “stop delay” test only compares invented counts and timestamps; it does not exercise these mechanisms.

**Fix:** Match sample counts to the actual transmitted interval, including the burst’s real start and already-handed-over tail. Remove the RX-packet proxy unless a justified TX-specific bound is established. Test both startup idle time and multiple packets sent beyond the stop cut.

#### 3. Major — Overflow produces a rig-dependent profile value

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:657–661,1457–1485`

The measurement promotes the full continuity gap after an intentionally long reader stall into `radio.timing.overflow_restart_gap_ns`. Its observation explicitly acknowledges that the value includes stall and socket-backlog effects.

**VERIFIED against UHD 4.10 source:** [`rx_streamer_zero_copy.hpp`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/transport/rx_streamer_zero_copy.hpp), `get_recv_buffs()`, drains buffered packets before invoking its overrun handler. [`rfnoc_rx_streamer.cpp`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_rx_streamer.cpp), `_handle_overrun()`, then requests restart. Recovery is therefore initiated through the receive path after draining—not independently while the reader sleeps.

Changing the imposed stall or host buffer changes the measured gap without changing the device’s recovery capability. The default 20-second stall can dominate the maximum and turn a test-rig parameter into the profile’s simulated overrun duration. Documenting that contamination does not make it a device measurement.

A single stall also does **not** establish exactly one overrun: additional sequence losses or recovery-time overruns remain possible. The exact-one check correctly makes such trials incomplete, but the defaults are still **INFERRED**, not a verified single-overrun recipe.

**Fix:** Keep these full gaps as observations and leave the comparable key unmeasured until the measurement separates delayed reader service from device recovery, or obtain an explicit design decision redefining the capability. P789 proves only the horizon arithmetic; it does not prove the chosen horizon completes a hardware capture.

### Other checks

- TX `tx_switch.at` records configuration completion and clock adoption, not a transmitted start (`provider/tx.rs:801–829`). The omitted burst criterion is explicitly permitted by the brief; this is limited readiness evidence, not burst-completion evidence.
- Candidate wiring, suffix/prefix selection, limiting-condition rows, fake-source replacement, and scoped file changes otherwise follow the brief.
- P776–P790 target real branches or record derivations, except that P789 cannot establish its claimed hardware adequacy.
- UR-34’s newly cited tests exist; no additional DA-1 defect identified.
- The reported **102 additional Runs/Sessions** is consistent with the loops. Its approximately 27 min 47 s estimate excludes startup/cleanup and earlier suite work.

Not checked: builds, tests, mutations, or hardware execution, as instructed. No files were changed.

## 修正の見直し（`sol-fix-3c1-review.log`，実装役が起こした）

### Prior Sol findings

1. **CLOSED — TX throughput lacks a usable end timestamp.**  
   **VERIFIED:** Throughput now uses the recorded burst target and exactly one timestamped `BurstAck`; it no longer requires `SampleClock.ended_at` (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1726–1744,1757–1763`). The synthetic success case explicitly uses an unended clock (`:2711–2726`).

2. **CLOSED — TX sample accounting compares mismatched intervals.**  
   **VERIFIED:** The Run advances to the scheduled burst start plus D, rather than clock origin plus D (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1795–1803`). Classification validates the recorded target, requires ACK ≥ start + D and `tx_samples` ≥ D × rate, and imposes no upper packet tolerance (`:1732–1763`). Observations retain the start, ACK, elapsed samples, actual samples and difference (`:1819–1828`). Synthetic tests cover startup idle time and excess handed-over samples (`:2711–2728`).

3. **CLOSED — Overflow promotes a stall-dependent gap.**  
   **VERIFIED:** The overflow group supplies observations but no comparable value (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1562–1577`); record construction consequently retains the default unmeasured row (`:181–206,231`). Restart estimates use gap end minus estimated reader resume, with negative results becoming null (`:1331–1358`). Per-rate and overall maxima are observation-only (`:1471–1494`). Positive, negative and summary cases have synthetic tests (`:2655–2705`); FakeDevice checks that overflow supplies no value row (`crates/ezsdr-radio-uhd/tests/fake.rs:685–708`).

### Remaining finding

#### 1. Medium — The TX test does not distinguish a short burst from a D-second burst

**VERIFIED:** The synthetic burst starts at root tick **2100**, and its required ACK boundary is **3100** (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:2711–2726`). Its only early-ACK case uses **1500**, before the burst even starts (`:2735–2736`). There is no case with an ACK after the start but before start + D.

**INFERRED regression risk:** Replacing the duration check with merely `ack >= burst_start_tick` would escape these assertions. Mutation P792 checks rejection of equality, not omission of D (`plan/maintenance/tools/mutations.json:11345–11353`).

Add an otherwise successful case with ACK **3099**, expecting `Incomplete`. This directly pins the adjudicated minimum-duration requirement. The current implementation’s comparison is correct (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1759`).

### Residual risks

- Hardware ACK timestamp availability remains unverified. The UHD adapter permits a missing timestamp, which this classifier safely treats as incomplete (`crates/ezsdr-radio-uhd/src/uhd.rs:1104–1116`; `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1742,1759–1762`).
- Overflow restart estimates and capture-horizon adequacy remain inferred, not hardware measurements (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1498–1518,1568–1570`).
- TX start-lead remains configuration-readiness evidence, explicitly not transmitted-burst evidence (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1180`).

No additional correctness defect found in the inspected brief wiring, record merging, synthetic/FakeDevice tests or nearby Provider paths. No files changed; no tests, mutations, hardware or agents run.

**VERDICT: CHANGES_REQUIRED**
