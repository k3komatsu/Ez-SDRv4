# 段階 3c-2b：Sol 6.1 のレビュー

## 指示書の事前の確認（`sol-brief-3c2b.md`）

### Recommendation: CHANGE all six procedures before implementation

**VERIFIED** below means checked against repository code or UHD 4.10.0.0 source—not demonstrated on hardware. UHD paths are relative to `/root/tmp/uhd-4.10.0.0/` in `ezsdr-v4-dev:uhd4.10`.

#### 1. H1 — CHANGE: distinguish timed fragmentation from untimed continuation

**VERIFIED:** A single timed 20,000-sample send does **not** timestamp only its first packet. UHD retains `has_time_spec` and advances the timestamp for subsequent fragments (`host/lib/include/uhdlib/transport/tx_streamer_impl.hpp:295–350`). FPGA defaults to packet-level error recovery: a late packet generates late-data error and is discarded; later untimed packets can transmit (`fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v:106–116,337–408`).

Keep the proposed timed-send case, but add a case with a deliberately small timed first send followed by explicit untimed continuation sends, EOB only on the final send. Otherwise it does not answer the maintenance table’s “untimed remainder” question.

Record each send’s length, accepted count, timestamp/SOB/EOB, all async reports including BurstAck, and **all** audible intervals—not merely their outer endpoints. `tx_async` is the correct report interface (`crates/ezsdr-radio-uhd/src/uhd.rs:1089–1116`; UHD `host/lib/rfnoc/radio_control_impl.cpp:1291–1317`).

#### 2. H2 — CHANGE: anchor the pause to actual playback

**INFERRED:** Sleeping 20 ms after submitting the head need not cause underflow: a sufficiently future start lets the tail arrive before the head finishes.

Schedule RX and the head first; use a pre-established device/host-time anchor to withhold the tail until after the head’s scheduled end plus a stated starvation interval. Record actual host send brackets and accepted counts. Classify trials without an observed Underflow separately.

**VERIFIED:** FPGA underrun returns to idle at a packet boundary, or discards the remainder of the interrupted packet; subsequent untimed data can resume (`radio_tx_core.v:359–408`).

Also, a 100 kHz tone at 1 MS/s repeats every **10 samples** (`hardware.rs:559–562`). Phase alone cannot yield an unambiguous signed sample displacement. Report phase/modulo-10 displacement, or use a uniquely correlatable waveform with synthetic ambiguity tests. Absolute head phase additionally includes the RF path’s phase.

#### 3. H5 — CHANGE: contiguous bursts evade the deadline being investigated

**VERIFIED:** Exactly adjacent Kernel bursts can continue one device burst. The held sample is sent without EOB before its continuation; the deadline-driven EOB path applies when no continuation is booked (`crates/ezsdr-radio-uhd/src/provider/tx.rs:335–359,425–452`).

Use 200 one-buffer bursts with explicit positive gaps and comfortably future reservations. Record requested/accepted starts, rejected submissions, actual burst boundaries, and async/event summaries. Keep contiguous bursts only as a labelled control.

**INFERRED:** This exercises isolated held-tail deadlines, but event counts alone cannot measure scheduling slack or prove every 2 ms deadline was met. Describe it as an idle-bench outcome observation, not a deadline measurement.

#### 4. #56, Stop between starts — CHANGE: verify device-call ordering and observe below publication

**VERIFIED:** `rx_start(e₂)` is issued during `Rx::begin`, before samples arrive (`crates/ezsdr-radio-uhd/src/provider/rx.rs:286–351`). FPGA STOP bypasses the RX command FIFO and clears queued commands (`fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v:171–183`).

Consequently, a Stop submitted between nominal e₁/e₂ does not establish whether it preceded or followed issuance of the device start.

Record planned e₁/e₂, actual `rx_start(e₂)` call brackets, Stop receipt and booking, and actual device stop calls. Classify the ordering retrospectively; do not claim deterministic placement from sleeps.

Manifest continuity demonstrates **published** samples and recovery, not necessarily everything the device streamed. A test-only forwarding Device observer can record received timestamps before Provider filtering. If no reader remains after the cut, absence of observations is inconclusive; add an explicitly labelled post-e₂ device-level drain before restarting, without concurrent readers. Capture recovery separately. Source expectation is cancellation of an already-queued start, but its bench outcome remains **INFERRED**.

#### 5. #56, Stop lag — CHANGE: the proposed timing row is unrelated

**VERIFIED:** `timing.stop.at/done` brackets `Provider::stop` and thread joins during shutdown (`crates/ezsdr-radio-uhd/src/provider/mod.rs:710–745`). A targeted Session Stop instead books a timeline item in uhd-control (`provider/control.rs:384–414`).

The clock domains are compatible in this device-primary Session: log receipt and Core timing use the primary root (`crates/ezsdr-kernel/src/coordinator/state.rs:305–315`; `provider/core.rs:926–942`). Units are **root ticks**, not nanoseconds.

Measure receipt-to-matched Stop booking, using the recorded plan item if identifiable. Device-stop issuance is a separate endpoint. If exact booking cannot be identified without new instrumentation, report that limitation rather than subtracting the shutdown row.

#### 6. UR-17 — CHANGE: separate buffering from acquisition and fault recovery

**VERIFIED:** `rx_recv` uses `one_packet=false`; UHD fills the request across packets until completion/error, with timeout applied to individual waits (`src/uhd.rs:983–995`; UHD `host/lib/include/uhdlib/transport/rx_streamer_impl.hpp:154–201`).

Record applied rate, packet size, timeout, per-call first/end timestamps, outcome and sample count; separate startup, steady-state and error calls. Buffered data can return quickly, so wall duration alone is not the VH-4 budget.

Add 390,625 S/s and a fault/overrun-near-cut observation: that is why the bound survived (`plan/maintenance/23-one-line-per-stream.md:129`). Healthy 1–50 MS/s calls cannot settle its removal.

**Safety/cleanup:** Keep the attenuated loopback and explicit waveform-amplitude checks; `raw_tx` checks frequency/rate/gain, not amplitude (`hardware.rs:333–338`). Stop/drain RX, finish finite TX, collect reports, close streamers and restore settings. Avoid device-time reads behind queued timed control writes; use prior anchors.

No edits, builds, tests or RF operations were performed. Hardware timing, report completeness and the assumed 44-sample delay remain unverified.

## コードのレビュー（`sol-3c2b.log`）

VERDICT: FAIL

### Scenarios checked

- H1: late timed fragments versus untimed continuation; partial sends, async reports and audible intervals.
- H2: starvation after playback begins; missing Underflow; absent, displaced or ambiguous PN matches.
- H5: adjacent versus separated bursts; whether held tails reach the deadline-driven path.
- #56: Stop before/after device-start issuance; post-cut drain; recovery; repeated Stop measurements.
- UR-17: buffered versus acquiring receives, startup/errors, and overrun near a cold cut.
- RF limits, forwarding-Device interference, cleanup, observation registration and scope.

### Findings

#### 1. Major — Neither #56 probe actually restarts the stopped receiver

**Locations:** `crates/ezsdr-radio-uhd/tests/hardware.rs:1820`, `:1911`.

Both probes “re-enable” reception by assigning `radio.rx.c0.stream = 0`. The channel already belongs to stream 0; Stop changes running state, not membership.

**Evidence:** `crates/ezsdr-radio-uhd/src/provider/control.rs:901–949` explicitly treats unchanged membership as a value-only update, without booking a new segment. The real restart operation is `start_rx`, handled at `control.rs:421–439`.

Consequently:
- The recovery capture cannot demonstrate a successful restart.
- After the first effective Stop, the remaining lag trials normally stop an already-stopped receiver. They time out without measuring another device-stop call.

**Fix:** Submit `start_rx` to `radio/rx/s0`. Record the corresponding new device-start call and recovery evidence before treating another Stop as a running-stream trial. Preserve unsuccessful starts as separate observations.

#### 2. Major — H5 mostly bypasses the deadline it is meant to exercise

**Location:** `crates/ezsdr-radio-uhd/tests/hardware.rs:1745–1763`.

The positive-gap case books all 200 bursts approximately one second ahead. Thus, when a burst’s last sample is held, its nonadjacent successor is already queued.

**Evidence:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:341–359` distinguishes:
- `Some(next nonadjacent burst)`: close the device burst immediately;
- `None`: wait until the held tail is a device lead away, then close.

Positive gaps prevent continuation, but do **not** force deadline-driven closure. Normally only the final burst exercises that wait; the adjacent control also ends with one such tail. Quiet event summaries therefore do not provide the requested 200 deadline-path observations.

**Fix:** Keep the prebooked cases as controls. For the deadline case, submit isolated bursts without a queued successor until each tail has closed. Record device-level TX send brackets and SOB/EOB metadata so the observation distinguishes immediate closure from deadline-driven closure. Continue describing the result as an idle-bench observation, not a measured deadline guarantee.

### Other checks

- H1/H2 use pretrial device-time anchors, explicit send records, amplitude checks and separate missing-underflow/ambiguous-correlation outcomes.
- The forwarding Device preserves forwarded arguments/results and excludes concurrent readers during its explicit drain. No additional forwarding defect was identified.
- The four synthetic tests are deterministic. P848–P851 target actual delay correction, offset arithmetic, ambiguity rejection and missing-code handling.
- All six probes are called and included in `hw_measurements_record`’s expected-name set. UR-34’s added test references exist and remain within its Test text.
- No production `src/`, measured-value, H3/H4, retry or fake-fault changes were found.
- Bench timing is provisional: the no-op restarts introduce roughly nineteen lag timeouts and three recovery-capture horizon waits, in addition to normal Session startup.

Not checked: builds, tests, mutation execution or hardware behavior; this review was read-only. The implementation report’s successful checks were not independently rerun.

## 修正の見直し（`sol-fix-3c2b.log`）

VERDICT: FAIL

#### 1. Major — Sol 2 OPEN: an unaccepted tail can unlock the next burst

**Evidence:** `crates/ezsdr-radio-uhd/tests/common/probe_device.rs:28–31` treats any numeric `accepted_samples`, including zero, as successful EOB. Its test explicitly accepts zero at `:206`. But the deadline closure sends a one-sample tail (`crates/ezsdr-radio-uhd/src/provider/tx.rs:553–579`).

If that send times out with `Ok(0)`, `hardware.rs:1817–1827` records `accepted_eob_before_next_submit = true` and permits the successor although the tail was not accepted.

**Fix:** Require acceptance of the complete recorded sample count, and test a one-sample EOB returning zero. Preserve incomplete closure separately and stop the serial sequence.

The structural correction otherwise works: controls remain (`hardware.rs:1775–1800`), and the serial case books no successor before the preceding EOB observation (`:1807–1827`). Sample counts, SOB/EOB, timestamps and host brackets distinguish a held-tail send from direct closure and expose the delay between the initial send and EOB.

#### 2. Note — Sol 1 CLOSED: restarts now reach the device

Both probes submit `start_rx` (`hardware.rs:1892`, `:1992`). **VERIFIED:** `src/provider/control.rs:433–439` books a new Start; `src/provider/rx.rs:286–332` selects the resulting segment and calls device `rx_start`.

Recovery capture requires successful device-start and subsequent sample evidence (`hardware.rs:1893–1901`; `common/probe_device.rs:33–37`). Stop-lag polling is bounded, counts only rows containing stop and recovery evidence, and ends after unsuccessful recovery (`hardware.rs:1984–2009`). No unbounded retry loop was found.

#### 3. Note — O1 CLOSED: drain panic resumes the reader

`common/probe_device.rs:22–25,80–81` installs a drop guard after pausing. The panic test checks that the reader is unpaused after unwinding (`:230–242`).

#### 4. Minor — Two mutation descriptions overstate their coverage

`plan/maintenance/tools/mutations.json:11985–12002`: P856 and P857 replace `>=` with `>`. These reject valid equality; they do **not** admit the stale samples their names describe.

**Fix:** Rename those rows and add mutations removing the ordering constraints. P852–P855 target EOB/restart predicates, and P858 targets guard removal, but none catches the incomplete-tail acceptance in finding 1.

#### 5. Note — TX instrumentation is not timing-neutral

`common/probe_device.rs:176–188` adds JSON allocation and a shared-row mutex on every send. `rows()` clones the accumulated log while holding that mutex (`:55–56`), competing with recording. This can delay the next TX turn; the device-call brackets exclude that post-call work.

No measured magnitude is available. Retain the instrumented, idle-bench limitation; establish recorder overhead before interpreting results as uninstrumented deadline behavior.

Not checked: builds, tests, mutations or hardware execution, as instructed. Reported successful checks were not independently rerun.

## 修正 2 の見直し（`sol-fix2-3c2b.log`）

VERDICT: PASS

- **1 — Major: CLOSED.** `crates/ezsdr-radio-uhd/tests/common/probe_device.rs:30–35` requires every recorded channel’s sample count to equal the accepted count. Tests at `:218–220` reject a one-sample EOB returning zero and a partially accepted send. `crates/ezsdr-radio-uhd/tests/hardware.rs:1820–1836` preserves incomplete closure evidence and stops before submitting another burst unless the EOB was fully accepted.

- **4 — Minor: CLOSED.** `plan/maintenance/tools/mutations.json:11985–12002` correctly describes P856/P857 as rejecting equality. P859/P860 (`:12015–12032`) remove the host-time and device-tick ordering checks; the stale-sample assertions at `crates/ezsdr-radio-uhd/tests/common/probe_device.rs:244–249` target those changes. P861 (`mutations.json:12035–12042`) disables counter advancement, directly contradicting the assertion at `probe_device.rs:234`.

- **5 — Note: CLOSED, with measurement limitation retained.** `crates/ezsdr-radio-uhd/tests/common/probe_device.rs:115–118` counts **every recorded EOB attempt**, including unsuccessful ones—not only accepted EOBs. This is correct: `crates/ezsdr-radio-uhd/tests/hardware.rs:1751–1761` waits for the corresponding attempt, then `:1820–1836` independently checks acceptance. Missing attempts time out; incomplete attempts stop the sequence. Polling no longer repeatedly clones the log. The observation at `:1878` explicitly states that recorder overhead is unmeasured and the result is not a held-sample deadline measurement.

No additional defect found in the reviewed sequences.

Not checked: builds, tests, mutation execution or hardware, as instructed. Instrumentation overhead and hardware behavior remain unmeasured.
