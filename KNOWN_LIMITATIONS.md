# Known limitations

Ez-SDR v4 is pre-release. This page lists small limits you can run into when you write an ExperimentSpec or a BindingProfile, use the Python client, or run on MockRadio, the simulator or a USRP X300/X310: limits that are known and deliberately not fixed, and a few known defects whose fix is scheduled. Defects tracked as GitHub issues are not listed here.

Each entry gives the **Symptom** (what you see), **When** it happens, a **Workaround**, the **Evidence** and its **Status**:

- *Ceiling* — a settled limit, stated in the accepted spec cited.
- *Device behaviour* — what the X300 or UHD does, VERIFIED on the bench; Ez-SDR records it or works around it.
- *INFERRED* — believed from source code or design, not yet observed.
- *Awaiting bench* — waits for the named bench check.
- *To be fixed in* — a known defect with a named Phase 8 stage.

Rule IDs (UR-7, RM-16, …) are stable and point into the accepted specs under `design/`; bench records are cited by file and line. Timing values are the `x3xx` 0.1.0 profile's as of 2026-10-27; Phase 8 stage 4 re-decides them from measurements. Entry numbers are never reused.

## USRP X300/X310 and the host

### KL-1 Opening the device sometimes fails: the reference PLL does not lock
- **Symptom:** connecting fails after about 30 s with `UR-7: the reference clock did not lock to its internal source within UHD's 30 s; connecting again usually succeeds (…)` (in Rust the error starts with the exported `REFERENCE_DID_NOT_LOCK`). Even a good open takes several seconds and varies (whole short bench tests took 4.5–24.6 s).
- **When:** on about 4 % of device opens with the internal reference (Phase 7: 2 of 63, 4 of 66, 3 of 68 opens; Phase 8: 2 of about 35).
- **Workaround:** connect again. The server already opens the device a second time by itself (selector `reopen_on_unlock`, default `true`) and records `timing { reopened_on_unlock, first_error }`; only a second failure reaches you.
- **Evidence:** UR-5, UR-7 (`design/18-uhd-radio.md`); `plan/phase7/bench-results.md:803, 1244, 1505`; `plan/phase8/bench-results.md:43, 90`.
- **Status:** Device behaviour. That a second open in the same process locks as a new process would is INFERRED: the automatic re-open has not yet been seen working on the bench.

### KL-2 Unplugging the 10 GbE link
- **Symptom:** the Run or Session ends `Stopped { policy { DEVICE_LOST } }` with its Manifest written, about 1.1 s after the link drops while receiving and about 3.0 s after in a transmit-only Session; Session calls made meanwhile wait until then. On a direct cable the process then aborts at exit (`terminate called after throwing an instance of 'uhd::io_error'`).
- **When:** the link to the X300 goes down during a Run (cable, switch port, device power).
- **Workaround:** none needed for the data: the Manifest is written before the abort. Through an L2 switch the process exited normally (INFERRED why). In Rust, opening the same `args` again in the same process once the link is back frees the kept device, and the process then exits normally.
- **Evidence:** UR-29 (`design/18-uhd-radio.md`); `plan/phase7/bench-results.md:1380, 1486–1497, 1520, 1535, 1584, 1616`.
- **Status:** Ceiling (UHD's static teardown, UR-29). Which UHD errors mean a lost device is INFERRED (UR-29).

### KL-3 After a retune: settling time and LO phase
- **Symptom:** a timed receive retune moves the tone 31–52 µs after its instant and has settled by about 60 µs. After an untimed tune the loop phase is random (spread 142°–180° over six retunes). A timed tune returned it to within 0.1° in Phase 7 and within 0.9° in BP5, but spread 99°–156° in three shake-out runs on 2026-10-27 (0.8° in the fourth), for a reason not known.
- **When:** OBX and UBX boards (`random_unless_timed_tune`); a CBX is `random`. A `cold` change tunes its stream's channels untimed when the new segment begins (UR-12), so phase coherence is lost there too (MockRadio models this, MR-34; on the USRP INFERRED).
- **Workaround:** drop about 60 µs after a timed retune; retune with `at` when phase matters and measure the phase after each retune; do not rely on phase across a `cold` change or on a CBX.
- **Evidence:** UR-9, UR-12 (`design/18-uhd-radio.md`); MR-34 (`design/09-mock-radio.md`); `plan/phase7/bench-results.md:437, 513–514`; `plan/phase8/bench-results.md:54, 101, 129`.
- **Status:** Device behaviour (settling and the untimed phase, OBX). Timed-tune repeatability is awaiting bench (Phase 8 stage 4).

### KL-4 Host and UHD setup
- **Symptom:** receive overflows or link drops at high rates; `uhd_find_devices` finds nothing just after the link comes up; UHD refuses the device (`Major compat number mismatch`); an OBX shows as an unknown board; the server refuses the USRP binding (`EA-7: ezsdr.radio.uhd: this server was built without UHD; …`).
- **When:** the host's NIC is not tuned (PCIe ASPM on, MTU below 9000, small socket buffers or NIC rings, NetworkManager managing the port), and these settings revert on reboot; the host's UHD does not match the X300's FPGA image; UHD older than 4.9, which does not know the OBX; an `ezsdr-server` built without its `uhd` feature (the default build).
- **Workaround:** check with `plan/phase8/tools/bench-host.sh --nic <nic> --subnet <n>` and apply with `--apply` as root after every reboot; UHD should log `Maximum frame size: 8000 bytes` (a large ping to the X300 going unanswered is normal); retry discovery; use UHD 4.10 with its FPGA image (`docker/uhd4.10/Dockerfile`) and build the server with `--features uhd`.
- **Evidence:** `plan/phase8/bench.md` (BP1, BP0); `handoff.md` §6.3; UR-9, UR-35 (`design/18-uhd-radio.md`); EA-7 (`design/16-easy-api.md`); `plan/phase8/bench-results.md:13, 19`.
- **Status:** Device behaviour (host and UHD requirements).

### KL-5 Transmit (and disk) throughput is below what admission allows
- **Symptom:** a sustained transmit at 100 MS/s underflowed after about 29 s and at 200 MS/s after about 3.3 s (`radio.TX_UNDERFLOW`), although admission allows 1 GB/s (250 MS/s) per direction on a 10 GbE link; 50 MS/s (200 MB/s) held in the same run.
- **When:** transmit above about 50 MS/s, bursts and repeats alike (a repeat is streamed from host memory, not replayed on the device). Captures written to disk at high rates have not been measured either: `cf32` takes 8 bytes per sample on disk (1.6 GB/s at 200 MS/s), and a capture that falls behind loses blocks as `link_drop` gaps (INFERRED for a slow disk).
- **Workaround:** keep transmit at or below 50 MS/s until stage 4 has measured it; watch for `TX_UNDERFLOW` and `link_drop` gaps. Receive alone ran 200 MS/s for 3 s with no overflow when nothing was written to disk.
- **Evidence:** RM-32 (`design/07-radio-model.md`); UR-9, UR-22 (`design/18-uhd-radio.md`); HD-10 (`design/10-host-data-path.md`); `plan/phase8/bench-results.md:111, 116–117`; `plan/phase7/bench-results.md:599`.
- **Status:** Awaiting bench (Phase 8 stage 4; one shake-out run so far, not used for the profile).

### KL-6 Configurations not yet tried on hardware
- **Symptom:** no bench evidence yet; for Basic/LF boards the Module refuses some tunes the board may accept.
- **When:** a CBX; BasicRX, BasicTX, LFRX or LFTX boards; mixed boards; more than one stream per direction; the XG image or two 10 GbE links; an external 10 MHz/PPS reference (`clock_source` or `time_source` not `internal`) and PPS-aligned starts. BasicRX/BasicTX tune up to 100 MHz (half the master clock; UHD's own range reaches 350 MHz), LFRX/LFTX within 0–32 MHz (UHD allows up to 64 MHz in its two-input modes), and their phase is declared the weaker `random_unless_timed_tune`.
- **Workaround:** check your configuration on a loopback (TX/RX → attenuator of 30 dB or more → RX2) before relying on it. The benches so far used an OBX and a UBX, the internal reference and one stream per direction.
- **Evidence:** §8 Deferred (`design/07-radio-model.md`); UR-7, UR-27 (`design/18-uhd-radio.md`); `plan/phase8/bench-results.md:30, 63`; `plan/phase8/00-overview.md` §6 (BP4b, BP4c).
- **Status:** Awaiting bench (BP4b, BP4c; Phase 8b for PPS distribution). The Basic/LF bounds are INFERRED.

## UHD Module: receive

### KL-7 A `cold` change interrupts the stream
- **Symptom:** changing a stream's sample rate, or a channel's antenna or stream membership, ends the stream's SampleClock at the change's instant; a new SampleClock starts later and nothing is delivered in between (not a gap: a new clock). On receive the pause is about 53 ms plus one receive call at the old rate (53.0 ms measured; about 55 ms at 1 MS/s with the default 2 000-sample block, derived; 220.8 ms measured at 390.625 kS/s with 65 536-sample blocks). On transmit, bursts booked on the old clock at or after the cut are cancelled (`radio.COMMAND_REJECTED`, "cancelled by a cold change"), and the new clock starts at least 50 ms later, so bursts aimed earlier are late.
- **When:** every `cold` change, on MockRadio's `x3xx` profile and on the USRP alike; also a receive restart after `stop()` and the first transmit enable from no channel.
- **Workaround:** make `cold` changes outside capture windows; set rates and channel sets in the BindingProfile or Spec before the Run; capture each configuration separately (KL-21).
- **Evidence:** RM-21, RM-25 (`design/07-radio-model.md`); UR-23, UR-25 (`design/18-uhd-radio.md`); MR-3 (`design/09-mock-radio.md`); `plan/phase7/bench-results.md:915–926, 1020`.
- **Status:** Ceiling (the pause is one receive call at the old rate, plus 3 ms, plus the 50 ms start lead). The start leads are INFERRED (shake-out candidate 1 ms; Phase 8 stage 4).

### KL-8 After a receive overflow, sample times are rounded
- **Symptom:** after `radio.RX_OVERFLOW { cause: overrun }`, blocks arrive with timestamps off the stream's sample lattice; the Module rounds each to the nearest sample and counts it in `stats.rx_off_lattice` (2 174–2 179 blocks in a 10 MS/s test).
- **When:** any receive overflow on the USRP; UHD restarts the stream itself, at an instant of its own choosing (within about 67–75 ms of the reader returning in the measurements so far).
- **Workaround:** avoid overflows (KL-4); after an `overflow_restart` gap, treat sample times as uncertain by up to half a sample.
- **Evidence:** UR-17, UR-18 (`design/18-uhd-radio.md`); `plan/phase7/bench-results.md:51, 259, 261`; `plan/phase8/bench-results.md:127`.
- **Status:** Ceiling (the rounding, UR-17). That UHD's restart instant is the cause is INFERRED.

### KL-9 Timed receive retunes on a stopped channel can wedge the X300 (D9 F1)
- **Symptom:** UHD logs `Control operation timed out waiting for space in command buffer`, later device calls fail, and the X300 needs a power cycle.
- **When:** about 32 or more frequency updates (`radio.rx.c<n>.frequency_hz`; every runtime frequency update is a timed command) to a receive channel while no samples pass its DDC: after a `Stop` of its receive stream, or while it waits for a later planned segment. The DDC applies timed values only as samples pass it, and the profile's `command_queue_depth` (16) stops counting an update once its instant has passed, so it does not stop the build-up.
- **Workaround:** none needed after D9a for the cases above; before it, retune a receive channel while its stream runs.
- **Evidence:** UR-24 (`design/18-uhd-radio.md`); design note 39 §1–§2 (`plan/phase8/39-dsp-timed-commands.md`); design note 42 §1 (`plan/phase8/42-d9-production-fix.md`); `plan/phase8/bench-results.md:89, 139`.
- **Status:** Fixed in stage D9a (2026-10-28) for the two paths above: a timed receive frequency now goes to the device only when a running segment covers its instant, and otherwise the next configure, or an untimed write once that configure has run, applies it (UR-24). Remaining, awaiting bench P4: a retune handed to the device just before a `Stop` or `cold` change cuts the stream short of its instant can leave one entry that the next segment's first samples take. A plan that covers the instant but whose stream never starts (a refused `rx_start`) can still leave entries (INFERRED).

### KL-32 A receive frequency set before the stream runs gives no phase guarantee (MA-1)
- **Symptom:** the loop phase after a receive frequency change is random from one Session to the next, where a timed tune would have given the same phase each time. MockRadio shows it too since stage MA1 (MR-34): a receive frequency that applies before a segment covers it, or while the stream is stopped, is an untimed tune there, with a fresh random phase.
- **When:** OBX and UBX (`random_unless_timed_tune`), for a receive frequency set (`radio.rx.c<n>.frequency_hz`) before T0, or while the channel's stream is stopped or waits for a later segment: the Module applies it untimed, because no sample passes the DDC at that instant (KL-9, UR-24). A frequency set with `at` while the stream runs is a timed tune and keeps the guarantee.
- **Workaround:** if the phase matters, in a Spec Run, schedule the frequency again (`update_parameter` with an `at`) after the receive stream has begun to flow, at an instant at least the release window (3 ms) after the segment's origin, and measure the phase after it (a Session's `set` carries no time).
- **Evidence:** UR-24 (`design/18-uhd-radio.md`); `plan/phase8/42-d9-production-fix.md`; `plan/phase8/reviews/d9a-opus.md` §5.
- **Status:** Ceiling (owner, 2026-10-28). MockRadio's MR-34 follows the same rule since stage MA1, conservatively: a frequency due less than the release window (3 ms) after the origin of the segment that covers it is untimed on the Mock, though the UHD Module may give it to the device timed. Awaiting bench: the probe `hw_ma1_phase_after_a_pre_start_tune` compares the spread of the two ways over N Sessions (an observation, not a proof of a guarantee).

## UHD Module: transmit

### KL-10 A transmit retune inside a burst or a repeat is half applied (D9 F3′)
- **Symptom:** the LO moves at the update's instant, but the DSP part of the tune (UHD's fine correction) keeps its old value until the next device burst, so the rest of the burst is off by the difference (its size depends on how UHD splits the tune; not measured).
- **When:** a timed `radio.tx.c<n>.frequency_hz` update whose instant falls after a burst's first send (an instant inside the first, time-stamped send is applied on time) (at most 2 000 samples by default and at most 2.5 ms): anywhere in a `repeat`, which is one device burst until it ends, and in bursts joined back to back into one device burst (KL-13).
- **Workaround:** retune transmit only between bursts, with a gap of at least 2 samples so that the next burst is a new device burst; to retune a repeat, stop it and start it again.
- **Evidence:** UR-22, UR-23 (`design/18-uhd-radio.md`); design note 42 §1 F3′ (`plan/phase8/42-d9-production-fix.md`); `plan/phase8/bench-results.md:140–141` (a DUC value applies only with time-stamped samples).
- **Status:** To be fixed in stage D9b after bench P6 (every send of a burst carries its time). VERIFIED from UHD 4.10's source and the Module's code; not yet seen on the bench. Pinned as a Ceiling by FakeDevice tests in stage D9a (`ur_33_a_timed_duc_value_is_taken_only_while_has_time_holds`).

### KL-11 Transmit retunes between bursts: leftovers (D9 S2) and pile-ups
- **Symptom:** (a) the first burst after a transmit channel re-joins its stream carries an older retune's DSP part instead of the latest one's; (b) after many retunes with no burst, values may apply at the wrong times.
- **When:** (a) a timed transmit retune whose instant falls between bursts, after which, before any burst on that channel, the channel leaves its stream (`radio.tx.c<n>.stream`), is retuned while out, and re-joins; (b) more than about 33 timed transmit retunes while no burst runs (the DUC holds them until time-stamped samples arrive and, when full, drops their times).
- **Workaround:** (a) let a burst run on the channel before it leaves, or retune only after it re-joins; (b) avoid long series of transmit retunes with no burst.
- **Evidence:** design note 42 §1–§2 (`plan/phase8/42-d9-production-fix.md`); `plan/phase8/bench-results.md:141` (a queued timed value overrides a later untimed one).
- **Status:** (a) Ceiling (owner, 2026-10-27), pinned by `ur_25_a_transmit_channel_that_leaves_and_returns_keeps_its_old_timed_value` (stage D9a). (b) INFERRED from UHD 4.10's source; not addressed by D9a or D9b.

### KL-12 The first timed transmit retune after opening can apply early (D9 F7)
- **Symptom:** a timed transmit retune takes effect when it is handed to the device, up to the 3 ms release window before its instant, instead of at the instant; a later one may then be early too.
- **When:** before the first burst after the device is opened (every Run or Session opens it and sets its time to zero), if the previous process's last transmit packet carried a timestamp later than the instant; visible only when the instant falls in the first ~3 ms of a burst.
- **Workaround:** start each Run with a short burst (zeros will do) before the first timed transmit retune (INFERRED to clear the stale state).
- **Evidence:** UR-7, UR-24 (`design/18-uhd-radio.md`); design note 42 §1–§2 (`plan/phase8/42-d9-production-fix.md`); `plan/phase8/bench-results.md:140` (P2: a DUC value given before the burst moved at the burst's start, about 19 ms early).
- **Status:** Awaiting bench P5; if confirmed, to be fixed in stage D9b (an untimed empty end-of-burst when the transmit streamer opens). The mechanism is VERIFIED from UHD 4.10's source; that it caused P2's early move is INFERRED.

### KL-13 Burst timing on the USRP
- **Symptom:** a burst meant to start exactly where another ends is late (per its LatePolicy it is moved a sample or two later, or dropped); a burst that preempts a running burst or repeat with too little lead is late; after `Stop` the transmitter keeps sending for 10–12 ms; an underflow ends the burst and the rest is not sent; on a loaded host a burst can end with `TX_UNDERFLOW`.
- **When:** (a) the continuation is booked after the first burst's last sample went out, about 2 ms before its end (the X300 drops a timed start at the tick its previous burst ended); (b) preemption with less than about 11 ms lead (the 10 ms in-flight window plus delivery); (c) `Stop` of a transmit stream: samples already handed over still go out; (d) any underflow; (e) uhd-tx not running within about 2 ms of a burst's end, when it sends the held last sample.
- **Workaround:** book back-to-back bursts at least 5 ms before the first ends, or leave a gap of 2 samples or more; preempt with 20 ms of lead or more; allow 12 ms after `Stop`; keep the host's load low.
- **Evidence:** RM-15, RM-16 (`design/07-radio-model.md`); UR-21, UR-22, UR-23 (`design/18-uhd-radio.md`); `plan/phase7/bench-results.md:550, 568, 583, 839, 882, 887–889, 903`; `plan/phase8/bench-results.md:50, 55, 100, 128`; `plan/maintenance/audits-2026-10.md` (post-Phase-7 table, item 4).
- **Status:** Device behaviour (gap 0 dropped, VERIFIED) and Ceiling (in-flight window, held last sample, abandoning the burst after an underflow). The one-sample restart gap is verified at 1 MS/s only and the 10 ms window is INFERRED; the held sample's 2 ms deadline awaits the owner's decision after bench H5.

## The `x3xx` profile (MockRadio and USRP)

### KL-14 With the HG image, rates above 25 MS/s are refused unless the link is declared
- **Symptom:** a Spec or update asking for more than 25 MS/s in one direction (summed over its channels) is refused (`RM-32: rx needs … B/s, one link carries 100000000`).
- **When:** an `x3xx` binding (MockRadio or USRP) whose selector has no `link_gbps`: with the default `image` `hg` the link is taken to be 1 GbE. Two links (`links: [2]`) do not raise the limit: it is one link's, whatever `links` says.
- **Workaround:** on the X300's 10 GbE port, add `"link_gbps": [10]` to the selector (1 GB/s, 250 MS/s summed per direction); the server's default profile (`connect()` without a profile) already does.
- **Evidence:** RM-28, RM-32 (`design/07-radio-model.md`); EA-9 (`design/16-easy-api.md`); `crates/ezsdr-radio/src/channels.rs:178`; `plan/phase8/bench-results.md:88`.
- **Status:** Ceiling (one link; two links await bench BP4c).

### KL-15 The timing values are still estimates
- **Symptom:** what is admitted, what is judged late and how long pauses last follow `x3xx` values that are mostly INFERRED. Phase 8 stage 4 will change them without a profile version change (no versions are bumped before the v4.0 freeze), so a Spec constrained on `radio.timing.*` may then be judged differently.
- **When:** always, on MockRadio's `x3xx` profile and on the USRP.
- **Workaround:** leave margin (command leads of 5 ms or more, preemption leads of 20 ms or more); re-check after stage 4.
- **Evidence:** §3, UR-9, UR-15, UR-24 (`design/18-uhd-radio.md`); MR-3 (`design/09-mock-radio.md`); `crates/ezsdr-radio/src/x3xx.rs` (`timing`); `plan/phase8/bench-results.md:49, 115–121, 127, 140`; `plan/phase7/bench-results.md:259, 399, 420, 452, 531`.
- **Status:** INFERRED; Phase 8 stage 4 re-decides them (`startup_latency_ns` is settled).

| Value | Profile | Seen on the bench so far |
|---|---|---|
| `min_timed_command_lead_ns` | 5 ms (USRP), 2 ms (MockRadio) | through the Module a burst 3 ms after submission is played and one 2 ms after is dropped as late; the raw device needs only 0.3–0.5 ms |
| `rx_start_lead_ns`, `tx_start_lead_ns` | 50 ms each | the device restarts on time 1 ms ahead; shake-out candidate 1 ms |
| `tx_in_flight_ns` | 10 ms | no underflow from 0.5 ms at 2 MS/s; shake-out candidate 3 ms |
| `release_window_ns` | 3 ms | enough in one 1 MS/s probe (`plan/phase8/42-d9-production-fix.md:17`) |
| `command_queue_depth` | 16 | the X300 queued 7 timed OBX tunes and the 8th call blocked until the queue drained, so more than about 7 timed updates due within 3 ms can stall the control thread (INFERRED) |
| `overflow_restart_gap_ns` | 50 ms | unmeasured (UHD's restart came at most ~67–75 ms after the reader returned) |
| `startup_latency_ns` | 2 s | settled: every Run's T0 is at least 2 s after it is armed |

## MockRadio and simulation

### KL-16 MockRadio is not yet the USRP's exact twin
- **Symptom:** an experiment that passes on MockRadio's `x3xx` profile can behave differently on the USRP.
- **When:** command leads between 2 and about 5 ms (MockRadio accepts 2 ms; on the USRP a burst needs about 3 ms from submission); loopback timing (MockRadio's transmit-to-receive delay is 45 samples at every rate; the X300's was 35, 44, 44 and 36 samples at 400 k, 500 k, 1 M and 2 MS/s); LO phase (MockRadio's phase after a timed retune is one constant per channel whatever the frequency; the X300's differs per frequency, −137.3° at 1 GHz and 30.6° at 1000.1234 MHz in one test); repeats (MockRadio refuses an odd length or more than 2^28 samples, RFNoC Replay's limits; the USRP streams repeats from host memory and takes them); refusals at a change's instant (the USRP refuses a `cold` change whose rate or antenna reads back differently; MockRadio only for an injected `cold_change_refused` fault).
- **Workaround:** use leads of 5 ms or more; calibrate delay and phase on the device; keep repeats of even length and at most 2^28 samples.
- **Evidence:** MR-3, MR-18, MR-34 (`design/09-mock-radio.md`); RM-23 (`design/07-radio-model.md`); UR-9, UR-12 (`design/18-uhd-radio.md`); `plan/phase8/bench-results.md:49, 51`; `plan/phase7/bench-results.md:460–497, 616`.
- **Status:** the lead and the path delay are to be levelled in Phase 8 stage 4; the phase constant and the refusal scope are Ceilings (MR-34, MR-18).

### KL-17 SimulationChannel is a simple propagation model
- **Symptom:** the simulated medium differs from RF in ways some experiments notice.
- **When:** a transmitter and a receiver tuned even 1 Hz apart hear nothing (frequency offsets are not modelled); between different sample rates, or for a delay that is not a whole number of transmit samples, the signal is held from one transmit sample to the next (zero-order hold), not interpolated; a delayed path is a delayed copy with a real gain, with no carrier phase from the delay; noise is one power per receiving radio, the same on each of its channels.
- **Workaround:** tune both ends to identical frequencies; for exact results use one shared rate and whole-sample delays; model offsets and phase in your own processing.
- **Evidence:** CH-4, CH-5, CH-8 and decisions C3, C4, C6, C10 (`design/11-simulation-channel.md`).
- **Status:** Ceiling (a later `sim` version may interpolate and add complex path gains).

### KL-18 Reproducibility caveats
- **Symptom:** (a) adding or moving a fault that loses samples changes the noise of every later sample on that channel; (b) the same Spec, BindingProfile and seed can differ in the last bits of noisy or scaled samples on another platform.
- **When:** (a) `noise_dbfs` together with an `rx_overflow` fault; (b) comparing Runs across machines or builds (the platform's `ln`, `cos`, `sin` and `powf`).
- **Workaround:** compare Runs with the same faults on one build and platform; compare across platforms with a tolerance.
- **Evidence:** CH-5, CH-11 (`design/11-simulation-channel.md`).
- **Status:** Ceiling.

### KL-19 A channel-coupled simulation runs at about real time
- **Symptom:** a SimulationChannel receiver at 1 MS/s costs about 1 s of wall time per simulated second in a debug build and about 0.15 s in a release build; the cost grows with the rate (one evaluation per sample).
- **When:** MockRadio in channel mode (a `sim.channel` section); a Run without a channel is much faster.
- **Workaround:** point `EZSDR_SERVER` at a release build of `ezsdr-server`; use lower rates or shorter Runs.
- **Evidence:** CH-4 and decision C11 (`design/11-simulation-channel.md`).
- **Status:** Ceiling (block-wise evaluation is the upgrade).

### KL-20 A simulation section naming an output is not caught at `validate`
- **Symptom:** a `sim.faults` entry whose `target` is an output id is accepted and never applied; a `sim.channel` coupling or `noise_dbfs` entry naming an output id passes `validate` and fails the Run at `arm`.
- **When:** the id names a Spec output instead of the radio's resource.
- **Workaround:** use the radio's Spec resource name in `target`, `tx`, `rx` and `noise_dbfs`; check the Manifest's events for each fault you planned.
- **Evidence:** SE-4, SE-5 (`design/08-simulation.md`); CH-2 (`design/11-simulation-channel.md`).
- **Status:** Ceiling.

## Python client and Sessions

### KL-21 `capture()` refuses a capture with a gap or a rate change
- **Symptom:** `capture(n)` raises `ezsdr.Error` ("… has a gap or an invalid stretch …", "… spans 2 SampleClocks …" or "… does not have one channel count …"), though the capture was written and is in the Manifest.
- **When:** the capture holds an overflow or link-drop gap, an invalid channel stretch, a rate change or a channel-count change.
- **Workaround:** call `request(n, at)`, wait for the `sink.CAPTURE_WRITTEN` event whose `action` is one of the request's dispatched ids (`wait_for`), then `read(artifact)` and split the bytes by the artifact's `continuity`.
- **Evidence:** EA-17 (`design/16-easy-api.md`); `python/ezsdr/session.py` (`samples`).
- **Status:** Ceiling (one array would hide the gap).

### KL-22 Where a capture starts
- **Symptom:** a capture can start later than its `at`, without an error, and `capture()` returns only the samples, not where they start.
- **When:** on the USRP, when the recorder already received the samples at `at` before the request reached it (an `at` in the past, or none); always while an earlier request is being served, since requests are served one at a time, each from the end of the one before. In simulation the Run's time moves with your calls, so `at` is met.
- **Workaround:** ask ahead, e.g. `capture(n, at=sdr.after(0.05))`; when the start matters, read the artifact's ContinuityMap (`first`) from the `sink.CAPTURE_WRITTEN` event after `request()`, or from the Manifest.
- **Evidence:** EA-17 (`design/16-easy-api.md`); HD-10 and decision H5 (`design/10-host-data-path.md`); `plan/phase7/bench-results.md:293–302` (captures that started on time; the late start is derived from the design, INFERRED).
- **Status:** Ceiling.

### KL-23 The getters show what was admitted, not what the device did
- **Symptom:** after a runtime refusal (`radio.COMMAND_REJECTED`, `radio.COMMAND_QUEUE_FULL`), `sdr.rx_channels[n].frequency`, `sdr.rx.sample_rate`, the other getters and `sdr.effective` still hold the refused value, and later admission checks judge that value.
- **When:** a Provider refuses an update after the Kernel admitted it.
- **Workaround:** watch `events()` for those kinds; the Manifest's `applied` section records what the device applied; issue the update again.
- **Evidence:** KC-27 (`design/06-kernel-coordinator.md`); EA-16 (`design/16-easy-api.md`).
- **Status:** Ceiling (feedback of applied values is later work).

### KL-24 Simulation only: child Runs and native components
- **Symptom:** on the USRP, `run(spec, …)` raises `ezsdr.Rejected` ("KC-37a: a device-paced Session runs no child Run …"), so `python/examples/sweep.py` cannot run there. Where native components exist (a Rust embedding that supplies them; the `ezsdr-server` binary ships none), a Spec with an `ezsdr.exec.native` Island fails at `prepare` on the USRP (`Unsupported`, "NX-4: … runs the Simulation class only"), and in simulation an Action addressed to a component (`UpdateParameter`, `SetTimer`, `Stop`) is admitted and then fails the Run ("NX-7: …").
- **When:** a device-paced Session or Run; a Session `set` or a schedule entry whose target is a component.
- **Workaround:** on hardware, sweep inside one Session (`set`, `capture`) or open one Session per step, and make reactive decisions in your client; give components their settings in the Spec.
- **Evidence:** KC-37a (`design/06-kernel-coordinator.md`); EA-14 (`design/16-easy-api.md`); NX-4, NX-7 and decision N4 (`design/14-native-executor.md`).
- **Status:** Ceiling until later phases (child Runs on hardware; Phase 10's execution engines).

## Specs, BindingProfiles and records

### KL-25 Spec resources are matched greedily
- **Symptom:** a Spec is refused (a constraint violation or `NodeAlreadyBound`) although another assignment of its resources to nodes would satisfy it.
- **When:** two or more Spec resources bind sub-resources of one instance (channels, streams) and a less constrained resource takes the node a more constrained one needs; resources are matched one at a time, in name order, without backtracking.
- **Workaround:** give the more constrained resource a name that sorts first (the implementation's order, not a spec rule).
- **Evidence:** SB-34 (`design/03-spec-and-binding.md`); `crates/ezsdr-kernel/src/plan/validation.rs` (`validate`) and `crates/ezsdr-kernel/src/plan/matching.rs` (`pick_node`).
- **Status:** Ceiling (bipartite matching is the upgrade).

### KL-26 What a Spec cannot schedule
- **Symptom:** (a) a Spec has no absolute device time: schedule entries are offsets from the Run's start; (b) a scheduled capture is refused (`Failed { arm }`, an ambiguous clock) when the radio's streams run at different rates.
- **When:** (a) v3 `onTime`-style starts at a device time; (b) Spec-scheduled captures with, for example, receive and transmit at different rates.
- **Workaround:** use a Session: Actions with `at` (`after`, `next_pps`) and `capture(n, at=…)`.
- **Evidence:** decision B4 (`design/03-spec-and-binding.md`); KC-16 and decision K7 (`design/06-kernel-coordinator.md`).
- **Status:** Ceiling.

### KL-27 Re-using a capture as an input
- **Symptom:** the input is refused (`Failed { plan }`, "KC-9: input <i>: …") when it is listed as its Manifest recorded it; two captures of two Runs can share an `id`.
- **When:** listing a Run's capture (with `partial`, `marks` or `continuity`) in another Spec's `inputs`.
- **Workaround:** list it with `partial: false` and empty `marks` and `continuity` (its `hash` still links it to the Manifest that made it), and rename one of two inputs that share an `id`.
- **Evidence:** KC-9 (`design/06-kernel-coordinator.md`); HD-10 (`design/10-host-data-path.md`).
- **Status:** Ceiling.

### KL-28 A Manifest does not identify the build
- **Symptom:** Manifests from different code revisions record the same Module versions and `impl_hash` values.
- **When:** always: `impl_hash` names the release (`ezsdr.radio.uhd 0.4.0`), not the build, and versions are not bumped before the v4.0 freeze.
- **Workaround:** keep the git commit (`git rev-parse HEAD`) and the server binary you ran with each result.
- **Evidence:** SE-8 (`design/08-simulation.md`); NX-3 (`design/14-native-executor.md`); UR-1 (`design/18-uhd-radio.md`); `AGENTS.md` §6.
- **Status:** Ceiling (a build hash is later work).

### KL-29 Edge cases in event and command records
- **Symptom:** (a) a Manifest's delivered events are ordered by path (hot-path events such as `RX_OVERFLOW` first), then by emission, not by time; (b) an `RX_OVERFLOW` dropped because the event ring was full is counted in `EVENTS_DROPPED` but does not mark the capture; (c) a timed update due just before the Run's end that the Module had not yet handed to the device is recorded as cancelled at the end.
- **When:** (a) always; (b) more hot-path events at once than the ring holds; (c) an update due a few ms before `Provider::stop` or a device loss, when the Module's own release was delayed.
- **Workaround:** (a) sort by each event's `time`; (b) take gaps from the capture's ContinuityMap; (c) keep timed updates clear of the Run's end.
- **Evidence:** RM-16, RM-24 (`design/07-radio-model.md`); UR-26 (`design/18-uhd-radio.md`).
- **Status:** Ceiling.

### KL-30 SigMF metadata is incomplete
- **Symptom:** the `.sigmf-meta` file has no `core:frequency` and no `core:datetime`; a capture that spans two SampleClocks or channel counts has no `.sigmf-meta` at all.
- **When:** every capture; the second case after a rate change or a channel move during the capture.
- **Workaround:** take the frequency from the Manifest (`applied`, the effective configuration) and the time from the artifact's ContinuityMap and the Manifest's `clocks`.
- **Evidence:** HD-15 and decision H3 (`design/10-host-data-path.md`); `crates/ezsdr-sink-capture/src/sigmf.rs` (`sigmf_meta`).
- **Status:** Ceiling.

### KL-31 Device time to UTC: a growing bound
- **Symptom:** converting a USRP sample time to UTC through the Manifest's relation carries an uncertainty that grows by 10 µs per second of Run time (100 µs per second to host time) and leaves out the host clock's own error against UTC.
- **When:** long hardware Runs time-stamped in UTC from the Manifest.
- **Workaround:** keep such Runs short, or keep your own time reference.
- **Evidence:** UR-8 (`design/18-uhd-radio.md`); decision T6 (`design/01-time-model.md`); `plan/phase7/bench-results.md:35` (device against host: about −5.3 ppm).
- **Status:** Ceiling (one relation, measured at the start); the drift bounds are INFERRED.

## Device behaviour the UHD Module already handles

- The X300 ignores the time of a receive stop (UHD 4.10's FPGA: "timed STOP commands are not supported"): the Module stops the stream untimed once its samples pass the cut and drops what follows; `timing` shows `rx_stop_untimed`. UR-17, UR-25; `plan/phase7/bench-results.md:338`.
- The X300 drops a burst whose timed start is the tick its previous burst ended: the Module continues the device burst there instead, so one `BurstAck` in the `async` section answers both bursts. UR-23; `plan/phase7/bench-results.md:550`.
- UHD sends an empty end-of-burst as one zero sample: the Module counts it (the next safe start is two samples later) and the burst record ends before it. UR-23; `plan/phase7/bench-results.md:882, 887–889`.
- UHD restarts an overrun receive stream by itself (within about 67–75 ms of the reader returning, so far): the capture shows an `overflow_restart` gap whose `lost` is its length. UR-18; `plan/phase7/bench-results.md:258–259`; `plan/phase8/bench-results.md:127`.
- Opening a front end on a second streamer while another holds it breaks the first and aborts the process in UHD: the Module never does this. UR-33; `plan/phase8/bench-results.md:73`.

