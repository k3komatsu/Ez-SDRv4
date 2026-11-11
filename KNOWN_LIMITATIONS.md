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

### KL-3 LO phase after a tune is random on OBX, UBX and CBX
- **Symptom:** after a tune the loop phase is random (spread 142°–180° over six untimed retunes on the bench); a timed tune returned it to within 0.1° in Phase 7 and within 0.9° in BP5, but spread 99°–156° in three shake-out runs on 2026-10-27 (0.8° in the fourth), for a reason not known. A receive tune settles within about 60 µs.
- **When:** OBX and UBX boards (`random_unless_timed_tune`); a CBX is `random`. Every tune is untimed: at connect, and again for each receive segment a `start_rx` begins, whether or not its values changed (UR-12; MockRadio models this, MR-34, redrawing the phase once per channel at each begun segment; on the USRP INFERRED). A timed tune has no form today.
- **Workaround:** measure the phase after each Session start and do not rely on phase across a `start_rx` or on a CBX; calibrate in your own signal processing.
- **Evidence:** UR-9, UR-12 (`design/18-uhd-radio.md`); MR-34 (`design/09-mock-radio.md`); `plan/phase7/bench-results.md:437, 513–514`; `plan/phase8/bench-results.md:54, 101, 129`.
- **Status:** Device behaviour (settling and the untimed phase, OBX). The timed retune of the earlier entry is gone (note 47): a receive value changes only on a stopped stream, untimed (KL-35); timed-tune repeatability has no form today.

### KL-4 Host and UHD setup
- **Symptom:** receive overflows or link drops at high rates; `uhd_find_devices` finds nothing just after the link comes up; UHD refuses the device (`Major compat number mismatch`); an OBX shows as an unknown board; the server refuses the USRP binding (`EA-7: ezsdr.radio.uhd: this server was built without UHD; …`).
- **When:** the host's NIC is not tuned (PCIe ASPM on, MTU below 9000, small socket buffers or NIC rings, NetworkManager managing the port), and these settings revert on reboot; the host's UHD does not match the X300's FPGA image; UHD older than 4.9, which does not know the OBX; an `ezsdr-server` built without its `uhd` feature (the default build).
- **Workaround:** check with `plan/phase8/tools/bench-host.sh --nic <nic> --subnet <n>` and apply with `--apply` as root after every reboot; UHD should log `Maximum frame size: 8000 bytes` (a large ping to the X300 going unanswered is normal); retry discovery; use UHD 4.10 with its FPGA image (`docker/uhd4.10/Dockerfile`) and build the server with `--features uhd`.
- **Evidence:** `plan/phase8/bench.md` (BP1, BP0); `handoff.md` §6.3; UR-9, UR-35 (`design/18-uhd-radio.md`); EA-7 (`design/16-easy-api.md`); `plan/phase8/bench-results.md:13, 19`.
- **Status:** Device behaviour (host and UHD requirements).

### KL-5 Transmit (and disk) throughput is below what admission allows
- **Symptom:** a sustained transmit at 100 MS/s underflowed after about 29 s and at 200 MS/s after about 3.3 s (`radio.TX_UNDERFLOW`), although admission allows 1 GB/s (250 MS/s) per direction on a 10 GbE link; 50 MS/s (200 MB/s) held in the same run.
- **When:** transmit above about 50 MS/s, bursts and repeats alike (a repeat is streamed from host memory, not replayed on the device). Captures written to disk at high rates have not been measured either: `cf32` takes 8 bytes per sample on disk (1.6 GB/s at 200 MS/s), and a capture that falls behind loses blocks as `link_drop` gaps (INFERRED for a slow disk).
- **Workaround:** keep transmit at or below 50 MS/s per channel, preferring one streamer per channel, until stage 4 has measured it; watch for `TX_UNDERFLOW` and `link_drop` gaps. Receive alone ran 200 MS/s for 3 s with no overflow when nothing was written to disk.
- **Evidence:** RM-32 (`design/07-radio-model.md`); UR-9, UR-22 (`design/18-uhd-radio.md`); HD-10 (`design/10-host-data-path.md`); `plan/phase8/bench-results.md:111, 116–117`; `plan/phase7/bench-results.md:599`.
- **Update 2026-10-28:** with uhd-tx at real-time priority (stage TXP), X300 #2 held 100 MS/s on one channel and 100 MS/s on each of two streamers (800 MB/s); 200 MS/s on one channel still failed, and two channels grouped in one streamer failed from 40 MS/s each (`plan/phase8/bench-results.md`, 2 channel record after TXP). Without it the same bench stopped at 10 MS/s.
- **Status:** Awaiting bench (Phase 8 stage 4: the profile value, and why a grouped transmit fails earlier than two streamers).

### KL-6 Configurations not yet tried on hardware
- **Symptom:** no bench evidence yet; for Basic/LF boards the Module refuses some tunes the board may accept.
- **When:** a CBX; BasicRX, BasicTX, LFRX or LFTX boards; mixed boards; more than one stream per direction; the XG image or two 10 GbE links; an external 10 MHz/PPS reference (`clock_source` or `time_source` not `internal`) and PPS-aligned starts. BasicRX/BasicTX tune up to 100 MHz (half the master clock; UHD's own range reaches 350 MHz), LFRX/LFTX within 0–32 MHz (UHD allows up to 64 MHz in its two-input modes), and their phase is declared the weaker `random_unless_timed_tune`.
- **Workaround:** check your configuration on a loopback (TX/RX → attenuator of 30 dB or more → RX2) before relying on it. The benches so far used an OBX and a UBX, the internal reference and one stream per direction.
- **Evidence:** §8 Deferred (`design/07-radio-model.md`); UR-7, UR-27 (`design/18-uhd-radio.md`); `plan/phase8/bench-results.md:30, 63`; `plan/phase8/00-overview.md` §6 (BP4b, BP4c).
- **Status:** Awaiting bench (BP4b, BP4c; Phase 8b for PPS distribution). The Basic/LF bounds are INFERRED.

## UHD Module: receive

### KL-7 A receive restart interrupts the stream
- **Symptom:** after `stop()`, `start()` begins a new SampleClock later and nothing is delivered in between (not a gap: a new clock). The pause is about 53 ms plus one receive call at the old rate (53.0 ms measured; about 55 ms at 1 MS/s with the default 2 000-sample block, derived; 220.8 ms measured at 390.625 kS/s with 65 536-sample blocks). After a restart the device refuses, the next `start_rx` begins its segment when it would have had the refused one run (RM-25): the pause is that start's own, once for each transition.
- **When:** every receive restart, on MockRadio's `x3xx` profile and on the USRP alike. Changing a stream's rate or its stream membership during a Run does not exist, and a channel's frequency, gain or antenna change only on a stopped stream, at the next start (KL-35); a restart's pause covers its configure (RM-25).
- **Workaround:** restart outside capture windows; set rates and channel sets at connect or in the Spec; capture each configuration in its own Session (KL-21).
- **Evidence:** RM-21, RM-25 (`design/07-radio-model.md`); UR-17, UR-25 (`design/18-uhd-radio.md`); MR-3 (`design/09-mock-radio.md`); `plan/phase7/bench-results.md:915–926, 1020`.
- **Status:** Ceiling (the pause is one receive call at the old rate, plus the 3 ms delivery allowance, plus the 50 ms start lead). The start leads are INFERRED (shake-out candidate 1 ms; Phase 8 stage 4). The `cold` change, the transmit cancellation by a change and the first transmit enable of the earlier entry are gone in note 47 stage 2 (2026-11-04).

### KL-8 After a receive overflow, sample times are rounded
- **Symptom:** after `radio.RX_OVERFLOW { cause: overrun }`, blocks arrive with timestamps off the stream's sample lattice; the Module rounds each to the nearest sample and counts it in `stats.rx_off_lattice` (2 174–2 179 blocks in a 10 MS/s test).
- **When:** any receive overflow on the USRP; UHD restarts the stream itself, at an instant of its own choosing (within about 67–75 ms of the reader returning in the measurements so far).
- **Workaround:** avoid overflows (KL-4); after an `overflow_restart` gap, treat sample times as uncertain by up to half a sample.
- **Evidence:** UR-17, UR-18 (`design/18-uhd-radio.md`); `plan/phase7/bench-results.md:51, 259, 261`; `plan/phase8/bench-results.md:127`.
- **Status:** Ceiling (the rounding, UR-17). That UHD's restart instant is the cause is INFERRED.

### KL-9 A receive DDC value can remain after bounded recovery (D9b R2′, L1)
- **Symptom:** the Run records `rejected` with `call: "rx_drain"`; a timed DDC value was not shown consumed by the normal drain or its one bounded flush and may still override a later receive configuration. L1 keeps the reported value in `left` on its stream chain until qualifying samples consume it. If the channel moves to another stream, that `left` stays on the old chain and can continue to block restoration. A failed `rx_restore` means the device's RX frequency may also differ from the value in force at `Provider::stop`.
- **When:** a handed timed receive frequency is not consumed by sample evidence before the drain deadline, the one bounded immediate-start/read/stop flush also fails to show consumption, or the device refuses the RX stop; restoration can independently fail. A moved channel may be physically consumed by its new stream while its software `left` remains on the old chain.
- **Workaround:** keep the channel on the same RX chain until a post-acceptance sample at or after the command instant consumes `left`; do not assume close/open clears the device's DDC. After a move, return the channel to the old chain for sample evidence or recover/power-cycle the device before trusting a later capture.
- **Evidence:** UR-24, UR-26 (`design/18-uhd-radio.md`); design note 39 §6.7 (`plan/phase8/39-dsp-timed-commands.md`); design note 44 §§3.4, 3.7 (`plan/phase8/44-device-chain-state.md`); FakeDevice P4 A/C (`ur_24_d9_p4_a_and_c_keep_a_pre_stop_ddc_entry_until_restart_samples`); L1 chain-reuse, restore, and move tests (ur_24_a_left_and_latest_survive_close_open_and_chain_reuse, ur_24_provider_stop_skips_restore_for_left_on_this_chain, ur_24_a_left_stays_on_the_old_chain_after_channel_move); hardware P4 A/C on UBX and OBX (`plan/phase8/bench-results.md`, 2026-10-29).
- **Status:** Gone in note 47 stage 2 (2026-11-04): no timed receive frequency is handed to the device, so no DDC value is left behind and no `rx_drain` or `rx_restore` runs (UR-24 is withdrawn). The FakeDevice DDC FIFO and the P4 A/C FIFO test named above went in note 48 S2a-i-β (2026-11-05). The entry stays for its number.

### KL-32 A receive frequency gives no phase guarantee (MA-1)
- **Symptom:** the loop phase after a receive frequency is random from one Session to the next, where a timed tune would have given the same phase each time. MockRadio shows it too (MR-34): every tune there is untimed, with a fresh random phase.
- **When:** OBX and UBX (`random_unless_timed_tune`), for every receive frequency: it is set at connect (before T0) or on a stopped stream before a `start_rx` (KL-35), so the Module applies it untimed, because no sample passes the DDC then (UR-12).
- **Workaround:** if the phase matters, measure it after each Session start and correct it in your signal processing; a timed retune has no form today.
- **Evidence:** UR-12 (`design/18-uhd-radio.md`); `plan/phase8/42-d9-production-fix.md`; `plan/phase8/reviews/d9a-opus.md` §5; bench 2026-10-28 (`plan/phase8/bench-results.md`): over 5 Sessions on X300 #1 with OBX, the loopback phase spread 164.5° when both frequencies were set before T0 and 2.3° when set again with `at` while streaming (the relative phase of the two LOs; the receive LO's share is not separated); that second case is gone (note 47).
- **Status:** Ceiling (owner, 2026-10-28).

### KL-33 A receive frequency set while the channel does not receive can be lost after a refused change
- **Symptom:** a timed receive frequency (`radio.rx.c<n>.frequency_hz` with `at`) never reaches the device: the samples after its instant keep the old frequency, while the configuration of record shows the new one.
- **When:** the frequency's instant falls where no receive segment of the channel runs, and a `cold` change or start before it is then refused so that the next segment begins earlier than planned, before the frequency's instant (RM-25 then removed the refused segment and advanced later origins). It needs the refusal to be booked after the frequency's release, so it is rare (INFERRED).
- **Workaround:** set the frequency again with `at` after the receive stream has begun.
- **Evidence:** design note 43 (`plan/phase8/43-refusal-keeps-origins.md`); the code path was verified by review (`plan/phase8/reviews/ma1-opus.md` §4).
- **Status:** Gone in note 47 stage 2 (2026-11-04): no `cold` change or timed receive frequency exists during a Run (RM-21), so none can be refused or lost. The entry stays for its number.

### KL-34 A delayed RX frequency can hold later timed commands (L1)
- **Symptom:** a timed RX frequency remains held until its covering stream has begun; commands behind it in the instance's release order, including another stream's RX update or a TX update, are delayed too. When the head is finally handed after `e`, UHD applies the past-due timed command on arrival, and its physical LO phase is not guaranteed.
- **When:** the RX owner has not published the covering segment as running when the release window opens, or an earlier held command blocks the ordered release. A held value may wait until the segment begins or the plan changes or ends.
- **Workaround:** arrange for the RX owner to begin its segment before the release window for the timed frequency, and avoid depending on a later command reaching the device while an earlier RX frequency is held. Schedule a fresh timed tune after the stream is running if phase matters; a Session `set` has no time.
- **Evidence:** UR-24 (`design/18-uhd-radio.md`); design note 44 §3.8 (`plan/phase8/44-device-chain-state.md`); ur_24_a_blocked_rx_head_holds_later_rx_and_tx_updates, ur_24_a_held_frequency_is_handed_after_its_instant, ur_24_a_release_waits_until_begin_publishes_eligibility.
- **Status:** Gone in note 47 stage 2 (2026-11-04): no timed receive frequency is held or released (UR-24 is withdrawn). The entry stays for its number.

## UHD Module: transmit

### KL-10 A transmit retune inside a burst or a repeat is half applied (D9 F3′)
- **Symptom:** the LO moves at the update's instant, but the DSP part of the tune (UHD's fine correction) keeps its old value until the next device burst, so the rest of the burst is off by the difference (its size depends on how UHD splits the tune; not measured).
- **When:** a timed `radio.tx.c<n>.frequency_hz` update whose instant falls after a burst's first send (an instant inside the first, time-stamped send is applied on time) (at most 2 000 samples by default and at most 2.5 ms): anywhere in a `repeat`, which is one device burst until it ends, and in bursts joined back to back into one device burst (KL-13).
- **Workaround:** none; every payload-bearing send is timed since D9b-1a, so a timed retune can take effect inside a burst.
- **Evidence:** UR-22, UR-23 (`design/18-uhd-radio.md`); design note 42 §1 F3′ (`plan/phase8/42-d9-production-fix.md`); `plan/phase8/bench-results.md:140–141` (a DUC value applies only with time-stamped samples).
- **Status:** Gone in note 47 stage 2 (2026-11-04): no transmit frequency is updated during a Run (RM-21), so none is half applied inside a burst. The entry stays for its number.

### KL-11 Transmit retunes between bursts: leftovers (D9 S2)
- **Symptom:** the first burst after a transmit channel re-joins its stream carries an older retune's DSP part instead of the latest one's.
- **When:** a timed transmit retune whose instant falls between bursts, after which, before any burst on that channel, the channel leaves its stream (`radio.tx.c<n>.stream`), is retuned while out, and re-joins.
- **Workaround:** let a burst run on the channel before it leaves, or retune only after it re-joins.
- **Evidence:** design note 42 §1–§2 (`plan/phase8/42-d9-production-fix.md`); `plan/phase8/bench-results.md:141` (a queued timed value overrides a later untimed one).
- **Status:** Gone in note 47 stage 2 (2026-11-04): no timed transmit retune exists, and a channel cannot leave or re-join a stream during a Run (RM-21). Stage 4 stages an untimed transmit value only after the channel's bursts end (RM-21, UR-26). The entry stays for its number.

### KL-12 The first timed transmit retune after opening can apply early (D9 F7)
- **Symptom:** a timed transmit retune takes effect early instead of at its instant: as soon as it is handed to the device, or at the start of the burst it falls in (about 19 ms early in the bench probes); a later one may then be early too.
- **When:** before the first burst after the device is opened (every Run or Session opens it and sets its time to zero), if the previous process's last transmit packet carried a timestamp later than the instant; visible only when the instant falls in the first ~3 ms of a burst.
- **Workaround:** none when the open-time flush is acknowledged. If `tx_flush` is rejected for a missing acknowledgement, the stale-header reset is not confirmed.
- **Evidence:** UR-7, UR-24 (`design/18-uhd-radio.md`); design note 42 §1–§2 (`plan/phase8/42-d9-production-fix.md`); `plan/phase8/bench-results.md:140` (P2: a DUC value given before the burst moved at the burst's start, about 19 ms early); bench 2026-10-28, probe P5 on X300 #2 with UBX: with a stale timed header the new value applied about 19 ms before its instant, without one it applied 25 µs after.
- **Status:** Gone in note 47 stage 2 (2026-11-04): no timed transmit retune is issued (RM-21); the open-time flush (UR-22) stays. The entry stays for its number.

### KL-13 Burst timing on the USRP
- **Symptom:** a burst meant to start exactly where another ends is late (per its LatePolicy it is moved to the safe start, a few samples later (15 after a short burst at 1 MS/s), or dropped); after `Stop` the transmitter keeps sending for 10–12 ms; an underflow ends the burst and the rest is not sent; on a loaded host a burst can end with `TX_UNDERFLOW`.
- **When:** (a) the continuation is booked after the first burst's last sample went out, about 2 ms before its end (the X300 drops a timed start at the tick its previous burst ended); (b) `Stop` of a transmit stream: samples already handed over still go out; (c) any underflow; (d) uhd-tx not running within about 2 ms of a burst's end, when it sends the held last sample.
- **Workaround:** book back-to-back bursts at least 5 ms before the first ends, or leave a gap of 4 samples or more after a burst of more than 32 samples (its payload end plus four), and `g` samples after a burst of 32 samples or fewer (`g` as KL-36 defines it); allow 12 ms after `Stop`; keep the host's load low.
- **Evidence:** RM-15, RM-16 (`design/07-radio-model.md`); UR-21, UR-22, UR-23 (`design/18-uhd-radio.md`); `plan/phase7/bench-results.md:550, 568, 583, 839, 882, 887–889, 903`; `plan/phase8/bench-results.md:50, 55, 100, 128`; `plan/maintenance/audits-2026-10.md` (post-Phase-7 table, item 4).
- **Status:** Device behaviour (gap 0 dropped, VERIFIED) and Ceiling (in-flight window, held last sample, abandoning the burst after an underflow). After a successful EOB close, the safe start is the closing send's exclusive end plus three samples, or plus max(3 samples, 15 µs) after a closed first send of at most 32 samples (RM-15, UR-22, KL-36; uhd-tx and FakeDevice follow the same rule). The hardware measurement is the 2026-11-10 sweep of both X300s from 0.1 to 200 MS/s (`plan/phase8/bench-results.md`): VERIFIED at the measured rates, and INFERRED at every interpolation not swept, which the 15 µs margin over the measured 10 µs maximum covers. The 10 ms window is INFERRED; the held sample's 2 ms deadline awaits the owner's decision after bench H5.

### KL-36 A burst booked right after a short burst is refused at receipt, not continued
- **Symptom:** a timed burst booked inside the region of an earlier finite burst is refused at receipt with `radio.COMMAND_REJECTED` (`UR-21: the burst overlaps an accepted booking`); it is not played as that burst's continuation. The reason is `UR-21: the burst overlaps an accepted booking` on the UHD Module and `MR-16: burst overlaps an accepted booking` on MockRadio. An untimed burst is not refused this way: it is placed after that burst (RM-15).
- **When:** a timed booking inside the region of a finite burst of 1 to 32 samples (RM-15): from its start up to its padded end plus `g − 1`, where `g` is 15 µs of samples at the stream's rate and at least 3 (15 at 1 MS/s, 30 at 2, 3000 at 200; a burst of 1 to 3 samples is padded to four). The Module sends such a burst as one closed device burst (UR-22, UR-23). Bursts of more than 32 samples continue as before: a booking at their exclusive end joins them when it is received in time (RM-15).
- **Workaround:** send one burst (its samples in one waveform), or start the next one `g` samples after the short burst's transmitted end, padding included: at 1 MS/s, for 1 to 3 samples that is start plus nineteen, for 4 to 32 samples end plus fifteen.
- **Evidence:** UR-22, UR-23 (`design/18-uhd-radio.md`); KL-13 (a); the first send's 32-sample floor (`plan/phase8/bench-results.md`, 2026-11-06); the post-burst gap measured on both X300s from 0.1 to 200 MS/s (`plan/phase8/bench-results.md`, 2026-11-10); issues #79, #81, #82. `rm_15_closed_short_gap_is_fifteen_microseconds_or_three_samples` and `s1_parity_short_head_region_is_end_plus_gap` pin the values. Untimed: `rm_15_an_untimed_booking_is_placed_after_the_booking_it_meets` (`crates/ezsdr-radio/tests/staging.rs`).
- **Status:** Ceiling (the decision of design note 48 D1, S3; not fixed by S3).

### KL-37 A burst booked before an earlier-starting burst is refused, though it overlaps nothing
- **Symptom:** a timed burst is refused at receipt with `radio.COMMAND_REJECTED`, reason `UR-21: the burst starts before a booking that may already be handed` on the UHD Module and `MR-16: burst starts before a booking that may already be handed` on MockRadio. Its samples overlap no accepted burst.
- **When:** the refused burst starts before an accepted burst that starts less than the in-flight window (`radio.timing.tx_in_flight_ns`) after the receipt that books the refused one. The Provider may already have handed that accepted burst to the device, so it cannot take an earlier burst ahead of it. A burst behind an open join point (a burst a following booking could still continue) does not count, since the device holds that burst until the join deadline. An untimed burst is not refused this way: it is placed after the accepted one (RM-15).
- **Workaround:** book the bursts in start order; a burst that starts after the accepted one is accepted.
- **Evidence:** RM-15 (`design/07-radio-model.md`), UR-21 (`design/18-uhd-radio.md`), MR-17 (`design/09-mock-radio.md`); the tests `rm_15_a_booking_before_a_chain_inside_the_hand_over_horizon_is_refused` and `ur_21_s5_b_handed_before_an_earlier_a_is_refused_at_receipt`; the parity test `s15_parity_b_first_accepted_booking_rule`.
- **Status:** Ceiling (by design: refusing the earlier burst keeps the device's samples in order when the owner may already have handed the later one).

### KL-38 After a refused configure, a short burst that follows it is closed, and a burst joined behind it is late
- **Symptom:** a timed burst's configure is refused by the device and the burst is removed. A short burst (32 samples or fewer) booked contiguously after it no longer continues into a later burst: it is closed at its standalone end. A burst booked contiguously behind that short burst is then decided by its late policy (`drop` flags `TIME_ERROR`, and it is not played), where it would have been played as its continuation. A piece accepted before that configure was refused may also fall to its late policy, when its command reaches the owner only after its join point.
- **When:** the device refused a burst's configure (`radio.COMMAND_REJECTED` for that burst), and the bursts booked after it are contiguous with it, as in a run of bursts with no gap.
- **Workaround:** none needed beyond the late policy: book the burst that must follow in the same waveform, or start it `g` samples after the short burst's end, padding included (`g` as KL-36 defines it).
- **Evidence:** RM-15 (`design/07-radio-model.md`, the one exception to the receipt-order rule); UR-22, UR-23 (`design/18-uhd-radio.md`); the tests `rm_15_a_short_head_left_alone_is_closed_and_takes_no_join` (`crates/ezsdr-radio/tests/staging.rs`) and `ur_21_s2_a_short_surviving_head_is_closed_and_a_piece_joined_behind_it_is_late` (`crates/ezsdr-radio-uhd/src/provider/tx.rs`).
- **Status:** Ceiling (by design: a short burst closed at its standalone end is the device framing UR-22 allows, and a refused configure is a device refusal that removes the burst).

### KL-39 After a burst is abandoned, a burst booked inside its unplayed rest is refused
- **Symptom:** a timed burst is refused at receipt with `radio.COMMAND_REJECTED` (`UR-21: the burst overlaps an accepted booking`), although the burst it overlaps stopped sounding earlier.
- **When:** the UHD Module abandoned a burst (UR-22: an underflow, a sequence error, a failed or short send, a continuation past its time), and the new burst starts inside what the abandoned burst still holds: a finite burst keeps its whole booked region; a repeat keeps its region up to the cursor it was abandoned at plus five samples (RM-15), which can be up to the in-flight window (`radio.timing.tx_in_flight_ns`) ahead of device time. MockRadio has no transmit fault model, so it never abandons a burst.
- **Workaround:** book the burst untimed (it is placed after the held region, RM-15), or after the abandoned burst's booked end; or send a `Stop` for the stream first, which cuts the held region at its bound (RM-16) but also cancels every burst booked after the Stop's instant.
- **Evidence:** RM-15 (`design/07-radio-model.md`), UR-22 (`design/18-uhd-radio.md`); `crates/ezsdr-radio/src/staging.rs` (`halt`: a chain started before the cut is kept up to the bound); design note 49 §5.4.4 item 2 and §12 (C6). The finite case is pinned by `o1_uhd_t7b_an_abandoned_finite_booking_keeps_its_whole_region` and the repeat's region by `o1_uhd_t7a2_the_region_of_an_abandoned_repeat_is_its_cursor_plus_five`; the `Stop` workaround is read from the code, not tested (INFERRED).
- **Status:** Ceiling (owner, 2026-11-09: kept as it is; shrinking the region to what was sent is the contract change C6 of design note 49, made only if the owner later approves it).

### KL-40 Transmit rates on the `x3xx` profile start at 1 MS/s
- **Symptom:** a transmit stream at a rate below 1 MS/s (0.5 MS/s, say) is refused: the key is `rejected` (`RM-8: 500000 is outside 1000000..200000000`) at `connect` or by a Spec's `requires`, and the rate is never coerced to 1 MS/s. Receive rates are unchanged (down to 98 kS/s), and so is the `ideal` profile.
- **When:** a transmit stream on `x3xx` (MockRadio and the UHD Module alike) at a rate whose decimation is above 200 (`master_clock_hz / N`, `N > 200`).
- **Workaround:** transmit at 1 MS/s or more and filter on the host.
- **Evidence:** the 2026-11-10 bench on both X300s (`plan/phase8/bench-results.md`): at 0.4 MS/s and below a closed burst of 4 samples or fewer underflowed itself, and a two-zero untimed flush was not acknowledged within 1 s at 0.25 MS/s and below. `rm_8_a_transmit_rate_below_one_msps_is_rejected_on_x3xx`, `mr_06_a_transmit_rate_below_one_msps_is_rejected_on_x3xx`, `ur_11_a_transmit_rate_below_one_msps_is_rejected`, `d3_the_two_providers_refuse_a_transmit_rate_below_one_msps_alike`.
- **Status:** Ceiling (owner, 2026-11-10: 0.5 MS/s was not swept finely, so the floor is the 1 MS/s that was).

### KL-41 A switch right after a delayed repeat is refused
- **Symptom:** a repeat switch (`radio.start_repeat` with the switch mark; Python's `tx.switch(x)`) is refused at receipt with `radio.COMMAND_REJECTED` (`UR-38: the repeat the switch replaces has not started on the device`); `switch()` raises `Rejected`, and no `switches` row is written. The refusal leaves the repeat it would replace as it was: it plays when its first send goes out, or is dropped at dispatch if it is late and its late policy says `drop`.
- **When:** on the UHD Module, when the switch arrives before uhd-tx has sent any of the repeat it replaces: the repeat's start has passed, but a host stall or the stream's configure held its first send back. A switch on a switch counts from the first send of that switch's own waveform. MockRadio never refuses it: its repeats play from their start in device time (MR-42).
- **Workaround:** retry the switch once the repeat sounds (a few milliseconds later); a repeat dropped at dispatch has to be started again. A switch booked well after the repeat's start is not affected.
- **Evidence:** UR-38 (`design/18-uhd-radio.md`), RM-37 (`design/07-radio-model.md`); design note 49 §12.1 (N2 = (b1), owner); `e2a_b1_a_switch_behind_an_unstarted_finite_burst_is_refused`, `o1_uhd_t8_a_switch_whose_repeat_the_host_has_not_issued_is_refused_at_receipt` and `e2a_84_a_second_switch_on_a_switch_the_host_has_not_issued_is_refused`; MockRadio's acceptance by `mr_42_a_switch_in_a_step_that_jumped_over_the_repeat_s_start_is_accepted`.
- **Status:** Ceiling (owner, 2026-11-10: a refusal the caller learns at once, rather than a switch accepted and dropped later).

### KL-42 A switch reads `applied` after a Stop although none of its waveform was sent
- **Symptom:** on the UHD Module, a `Stop` read after an accepted repeat switch whose boundary `b` is before the Stop's bound `B` leaves the switch's `switches` row `applied`, yet no sample of the new waveform is sent: its payload is cancelled with a `UR-26: cancelled by Stop` note. MockRadio sends the new waveform from `b` up to its payload cut for the Stop (MR-25), which ends before `B`.
- **When:** uhd-tx has not handed past `b` when it reads the Stop: a host stall, or a `Stop` sent right after the switch.
- **Workaround:** read the burst records: no record carries the switch's waveform.
- **Evidence:** UR-38 (`design/18-uhd-radio.md`), RM-37 (`design/07-radio-model.md`); `e2a_s11_a_stop_after_a_switch_whose_boundary_is_before_the_bound_applies_it_and_cancels_the_payload`; design note 49 §5.4.5, the table row "Stop が b の操作の発行の前に来る".
- **Status:** Known defect; fixed in design note 49 stage C2 (a switch's outcome follows whether the sample at its boundary was sent).

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
- **Workaround:** leave margin (command leads of 5 ms or more); re-check after stage 4.
- **Evidence:** §3, UR-9, UR-15, UR-24 (`design/18-uhd-radio.md`); MR-3 (`design/09-mock-radio.md`); `crates/ezsdr-radio/src/x3xx.rs` (`timing`); `plan/phase8/bench-results.md:49, 115–121, 127, 140`; `plan/phase7/bench-results.md:259, 399, 420, 452, 531`.
- **Status:** INFERRED; Phase 8 stage 4 re-decides them (`startup_latency_ns` is settled).

| Value | Profile | Seen on the bench so far |
|---|---|---|
| `min_timed_command_lead_ns` | 5 ms (USRP), 2 ms (MockRadio) | through the Module a burst 3 ms after submission is played and one 2 ms after is dropped as late; the raw device needs only 0.3–0.5 ms |
| `rx_start_lead_ns` | 50 ms | before note 48 B3 (the receive restart lost its release-window term, 2026-11-06): the device restarted on time 1 ms ahead, shake-out candidate 1 ms, receive candidates on time from 3 ms. These do not show the margin left after B3; measure again before lowering the profile lead |
| `tx_in_flight_ns` | 10 ms | no underflow from 0.5 ms at 2 MS/s; shake-out candidate 3 ms |
| `overflow_restart_gap_ns` | 50 ms | unmeasured (UHD's restart came at most ~67–75 ms after the reader returned) |
| `startup_latency_ns` | 2 s | settled: every Run's T0 is at least 2 s after it is armed |

## MockRadio and simulation

### KL-16 MockRadio is not yet the USRP's exact twin
- **Symptom:** an experiment that passes on MockRadio's `x3xx` profile can behave differently on the USRP.
- **When:** command leads between 2 and about 5 ms (MockRadio accepts 2 ms; on the USRP a burst needs about 3 ms from submission); loopback timing (MockRadio's transmit-to-receive delay is 45 samples at every rate; the X300's was 35, 44, 44 and 36 samples at 400 k, 500 k, 1 M and 2 MS/s); LO phase (MockRadio's phase after a timed retune is one constant per channel whatever the frequency; the X300's differs per frequency, −137.3° at 1 GHz and 30.6° at 1000.1234 MHz in one test); repeats (MockRadio refuses an odd length or more than 2^28 samples, RFNoC Replay's limits; the USRP streams repeats from host memory and takes them); refusals at a start (the USRP refuses a `start_rx` whose rate or antenna reads back differently; MockRadio never).
- **Workaround:** use leads of 5 ms or more; calibrate delay and phase on the device; keep repeats of even length and at most 2^28 samples.
- **Evidence:** MR-3, MR-18, MR-34 (`design/09-mock-radio.md`); RM-23 (`design/07-radio-model.md`); UR-9, UR-12 (`design/18-uhd-radio.md`); `plan/phase8/bench-results.md:49, 51`; `plan/phase7/bench-results.md:460–497, 616`.
- **Status:** the lead and the path delay are to be levelled in Phase 8 stage 4; the phase constant and the refusal scope are Ceilings (MR-34, MR-18). MockRadio's receive configure takes no time, so it never reports `radio.START_LEAD_SHORT`, which the USRP's owner emits when a start's configure leaves less than the declared command lead before its origin (RM-25).

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

### KL-21 `capture()` refuses a capture with a gap or a restart
- **Symptom:** `capture(n)` raises `ezsdr.Error` ("… has a gap or an invalid stretch …", "… spans 2 SampleClocks …" or "… does not have one channel count …"), though the capture was written and is in the Manifest.
- **When:** the capture holds an overflow or link-drop gap, an invalid channel stretch, a `stop()` and `start()` (a new SampleClock) or a channel-count change.
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
- **Symptom:** after a device refusal at a restart (`radio.COMMAND_REJECTED`), `sdr.rx_channels[n].frequency`, `sdr.rx.sample_rate`, the other getters and `sdr.effective` still hold the value asked for at connect, and later admission checks judge that value.
- **When:** the device refuses the configuration a `start_rx` applies (UR-12), after the Kernel admitted the value at connect.
- **Workaround:** watch `events()` for `radio.COMMAND_REJECTED`; the Manifest's `applied` section records what the device applied; `start()` again or connect with another value.
- **Evidence:** KC-27 (`design/06-kernel-coordinator.md`); EA-16 (`design/16-easy-api.md`).
- **Status:** Ceiling (feedback of applied values is later work).

### KL-24 Simulation only: child Runs and native components
- **Symptom:** on the USRP, `run(spec, …)` raises `ezsdr.Rejected` ("KC-37a: a device-paced Session runs no child Run …"), so `python/examples/sweep.py` cannot run there. Where native components exist (a Rust embedding that supplies them; the `ezsdr-server` binary ships none), a Spec with an `ezsdr.exec.native` Island fails at `prepare` on the USRP (`Unsupported`, "NX-4: … runs the Simulation class only"), and in simulation an Action addressed to a component (`UpdateParameter`, `SetTimer`, `Stop`) is admitted and then fails the Run ("NX-7: …").
- **When:** a device-paced Session or Run; a Session `set` or a schedule entry whose target is a component.
- **Workaround:** on hardware, open one Session per step, with the step's values at connect (KL-35), and make reactive decisions in your client; give components their settings in the Spec.
- **Evidence:** KC-37a (`design/06-kernel-coordinator.md`); EA-14 (`design/16-easy-api.md`); NX-4, NX-7 and decision N4 (`design/14-native-executor.md`).
- **Status:** Ceiling until later phases (child Runs on hardware; Phase 10's execution engines).

### KL-35 Most radio values change only on a stream that is not running
- **Symptom:** a channel's frequency, gain and antenna change only while their stream is not streaming (a transmit channel's, while its bursts do not hold it): `sdr.rx_channels[n].frequency = …` (and `gain`, `antenna`) on a streaming stream raises `ezsdr.Rejected` whose `event` is the Provider's `radio.COMMAND_REJECTED` ("… stream 0 is streaming; send Stop first …"), while `sdr.rx_channels[n].frequency` still reads the value the Kernel admitted (KC-27), which the stream does not use; a start whose configure the device refused leaves the stream counted as streaming, so a corrected value is refused until `Stop`; so is a `start_rx` (`rx.start()`) of a stream that has not been stopped or whose start is already booked (a booked start is never replaced by an earlier one): `Rejected` with the Provider's event, until `Stop`. `sdr.rx.sample_rate`, `sdr.tx.sample_rate` and `stream` raise `ezsdr.Error`; a raw `sdr.set` of one is `Rejected` with the check `ezsdr.update_class`; a Spec `schedule` entry of a receive value passes `validate` and is refused by the Provider at run time (it carries an instant), and one of any other radio key fails `validate`.
- **When:** every Run and Session, on MockRadio and on the USRP. A receive value is accepted on a stopped stream, staged, and used by the next `start_rx` (`rx.stop()`, then `rx.start(frequency={…}, gain={…}, antenna={…})`); a transmit value is accepted while its channel's bursts do not hold it (RM-21); the rate, in either direction, waits for a bench confirmation on a held-open streamer; a channel's stream membership is fixed at connect.
- **Workaround:** send `Stop`, then the values (or `rx.start(frequency=…)`, which sends them before the start), then the start; after a refused start, `Stop` first again; read what the stream uses from the capture, not from the attribute after a refusal; set the rate and membership at connect or in the Spec's `requires`; a transmit value waits for the channel's bursts to end (after a `Stop`, for the in-flight window); for a sweep of them open one Session per step (KL-24).
- **Evidence:** RM-21, RM-27 (`design/07-radio-model.md`); MR-18 (`design/09-mock-radio.md`); UR-26 (`design/18-uhd-radio.md`); EA-11, EA-16 (`design/16-easy-api.md`); RS-52 (`design/04-run-and-session.md`); SB-16 (`design/03-spec-and-binding.md`); `plan/phase8/47-no-change-while-streaming.md`.
- **Status:** Ceiling for the rate, in both directions (until the bench confirms a rate change on a held-open streamer), and for a transmit value's wait: after a `Stop` the value is refused until the in-flight window (`radio.timing.tx_in_flight_ns`, 10 ms on x3xx) has passed and three samples more, which can be longer than the air needs (RM-21, RM-16). The bench group that measures a restart's configure against `rx_start_lead_ns` (compile-only bench code) is not restored: a named follow-up of the bench step (note 47, work order 3) before any profile `rx_start_lead_ns` changes, and until then a USRP may report `radio.START_LEAD_SHORT`. A receive retune on a held-open streamer is verified on the fake device only; the bench has to show that the untimed tune after a `Stop` is accepted and that RF settling fits between `apply` and the first sample (`plan/phase8/reviews/47s3-test-plan.md` H1).

### KL-43 Departures the server cannot see, and a server that cannot be stopped from the client
- **Symptom:** the server ends its Session at once when its input ends, also inside a request (EA-15), but it learns only of what reaches its input, and a few departures do not. (a) A silent network partition over ssh (a laptop asleep, Wi-Fi or a VPN lost, no FIN or RST): the server's input does not end until sshd gives up on the client, and the transmit goes on meanwhile. (b) A signal sent to `ezsdr-server` itself (`kill`, `docker stop`, the OOM killer), or its death: no cleanup and no Manifest; the transmit stops only when the device runs out of samples (INFERRED: a repeat is host-fed, KL-5; how the X300 behaves after the host process dies is to be measured, and LO leakage may remain). (c) The server runs in a session of its own, so Ctrl-C reaches only the Python client: a server hung where it cannot see its input end (a module call that does not return, a reply write to a pipe that a client still alive does not read) is no longer killed by Ctrl-C; Python waits 30 s for it and then kills it, and a second Ctrl-C during that wait ends Python and leaves the server running; a server behind ssh is never killed by the client (it kills its local ssh). (d) A process that inherited the client's pipe to the server (`os.fork`, a `multiprocessing` fork worker) and is still alive keeps the server's input open: no end of input. (e) After a call that was interrupted, `close()` waits for the server and returns the Manifest only if it can read `<dir>/manifest.json`: it returns `None` when the server runs on another host, and when the server was killed after the 30 s wait (once a Session exists, every exit of the server writes the Manifest, so a Session whose server ended by itself has one). An interrupted `connect()` returns only after the server has finished `connect` and exited, up to 30 s, and has no Manifest when the server had not begun the request: its frame was not yet read, so there is no Session directory at all.
- **When:** (a) remote use with sshd's default `ClientAliveInterval 0`, when TCP keepalive decides (about 2 h 11 min with Linux defaults, INFERRED); (b) a signal sent to the server rather than to its client; (c) a hung server, or a server behind ssh, and Ctrl-C; (d) a forked helper holding the pipe; (e) a server on another host, or a kill.
- **Workaround:** on the host that runs the server over ssh, whichever it is, set `ClientAliveInterval` and `ClientAliveCountMax` in its sshd (for example 5 and 3, so a silent client is dropped after about 15 s); end a Session by stopping its client (Ctrl-C, `kill <client pid>`), never by signalling the server; for a stuck server, stop it on its own host by hand (`kill <server pid>`, `docker kill`), and check the device is quiet; do not fork a client that holds a Session, or close the inherited pipe in the child.
- **Evidence:** EA-15 and EA-16 (`design/16-easy-api.md`); KC-36 (`design/06-kernel-coordinator.md`); `plan/phase8/bench-results.md` "2026-11-09" (i) for a drop sshd sees; `plan/phase8/bench.md` "#89 の後" for the bench that measures the server's death.
- **Status:** Ceiling (owner, 2026-11-11: a transport's liveness is the transport's; no heartbeat, no signal handler). INFERRED: sshd's behaviour on a silent partition, and the stop by starvation. A server-side backstop that exits N seconds after the input ended is to be decided after the bench measurement of (b).

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
- **Symptom:** (a) a Manifest's delivered events are ordered by path (hot-path events such as `RX_OVERFLOW` first), then by emission, not by time; (b) an `RX_OVERFLOW` dropped because the event ring was full is counted in `EVENTS_DROPPED` but does not mark the capture; (c) gone in note 47 stage 2: no timed update exists.
- **When:** (a) always; (b) more hot-path events at once than the ring holds; (c) none.
- **Workaround:** (a) sort by each event's `time`; (b) take gaps from the capture's ContinuityMap; (c) none.
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
- The X300 drops a burst whose timed start is the tick its previous burst ended: the Module continues the device burst there instead, except after a short burst that was padded and closed: a booking at that burst's payload end or before its padded front is decided by its own late policy (UR-22). Otherwise one `BurstAck` in the `async` section answers both bursts. UR-23; `plan/phase7/bench-results.md:550`.
- The first send of a device burst that stays open carries at least 32 samples per channel, and a closed one (its end in the same send) at least four. A closed burst shorter than four is padded with zeros after its samples and sent at once, so the device transmits zeros the experiment did not ask for; the next safe start is the exclusive end of the sent device burst, padding included, plus three samples, or plus `max(3, ceil(15 µs × rate))` samples (15 at 1 MS/s) after a closed first send of at most 32 samples (UR-22; the 2026-11-10 sweep, VERIFIED at the rates measured, INFERRED between them). The X300 failed the shorter closed first sends measured at 1 and 2 MS/s, and the open first sends of 28 samples or fewer at interpolation 80 or more (2026-11-06, `plan/phase8/bench-results.md`); the floors apply to every X3x0 profile. The burst record counts payload only. UR-22, UR-23; `plan/phase7/bench-results.md:882, 887–889`.
- UHD restarts an overrun receive stream by itself (within about 67–75 ms of the reader returning, so far): the capture shows an `overflow_restart` gap whose `lost` is its length. UR-18; `plan/phase7/bench-results.md:258–259`; `plan/phase8/bench-results.md:127`.
- Opening a front end on a second streamer while another holds it breaks the first and aborts the process in UHD: the Module never does this. UR-33; `plan/phase8/bench-results.md:73`.
