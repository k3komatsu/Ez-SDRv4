# Phase 7 — bench results

The record of the bench session (GZ-10) that [`bench.md`](bench.md) describes. Session 1: 2026-09-30, run from a Claude Code session on the owner's Mac over `ssh usrp-lnx02`, in the Docker image of `docker/uhd4.10/Dockerfile`. Every test's whole output is on the server in `~/ezsdr-bench/<test>.log`.

## The bench

| | |
|---|---|
| Device | **USRP X300** (not an X310: bench.md says an X300 serves), serial 347D545, motherboard revision 13 (compat 7) |
| FPGA image | HG, version 39.3, git hash `d375d68` — UHD 4.10's image, loaded on 2026-09-30 with `uhd_image_loader --args type=x300,addr=192.168.40.36,fpga=HG` (the X300 had UHD 4.6's image, whose `0/Radio#0` revision 0 UHD 4.10 refuses), then power-cycled |
| Daughterboards | slot A **OBX** (TX ID 0x3500, RX ID 0x3501: `OBX TX`, `OBX RX`, 10–8400 MHz, 0–31.5 dB in 0.5 dB steps, 160 MHz, antennas TX `TX/RX`, `CAL`; RX `TX/RX`, `RX2`, `CAL`); slot B empty (`Unknown (0xffff) - 0`) |
| RF | OBX TX/RX → SMA + 30 dB attenuator → OBX RX2 |
| Host | `usrp-lnx02`: Ubuntu 24.04.4 LTS, kernel 7.0.0-31-generic x86_64, 20 cores; NIC Intel X710 (`i40e`) port 0 `enp2s0f0np0`, 10 Gb/s, `192.168.40.10/24`, MTU 9000 (NetworkManager profile "USRP 10Gb(40)"); `net.core.rmem_max` 50 000 000, `wmem_max` 33 554 432 |
| UHD | 4.10.0.0 in the Docker image (the host's apt UHD 4.6 does not know the OBX) |
| `ARGS` | `addr=192.168.40.36` (the X300's address on its 10 GbE port 1; not 192.168.40.2) |
| Commit | `99545d5` on `worktree-phase7-impl` |

Set-up findings: the cable was first not seated, then in the NIC's port 1 (whose profile is `192.168.44.10/24`, a subnet the X300 is not on: UHD's broadcast found it, unicast did not reach it); in port 0 it works. The profile of port 1 also carries a default gateway `192.168.44.1` (metric 20101), harmless while the LAN's route wins.

## B0 — set-up (no RF): pass

In the image on `usrp-lnx02`, native x86_64: `cargo test --workspace` 847 passed on stable and on 1.85.0 (the three wall-clock tests that failed under the Mac's amd64 emulation pass here); `cargo test -p ezsdr-radio-uhd --features uhd`: fake 97, `uhd_api` 4 (the struct sizes against libuhd 4.10 on x86_64), hardware 10 ignored; clippy `-D warnings` with `uhd` clean; `cargo build --release -p ezsdr-server --features uhd` built; Python 23 tests OK (3.14). `uhd_usrp_probe` exits 0 with no warning.

## B1 — `hw_b1_probe`: pass

- `B1 profile: x310-obx`; rx and tx 2 channels each, front ends `OBX RX`/`OBX TX` on channel 0 and `Unknown (0xffff) - 0` on channel 1 (slot B), as `one_obx` fakes it.
- Time advanced 20 054 952, 20 110 261, 20 111 131 ticks per 100 ms (pass: 20 000 000 ± 1 %; the excess is the sleep's).
- `ref_locked`: `Some(false)` with the internal reference. Neither UR-13 nor UR-27 reads it for `internal`; recorded for an external reference later.
- What a Session inherits if nothing configures the transmitter (UR-25): tx 0 rate 200 000 000, frequency 10 000 000.000000238 Hz, gain 0 dB.

## B2 — `hw_b2_authority`: pass

- `next_wakeup` lateness: median 557 µs, max 664 µs (pass: < 1 ms, < 25 ms).
- Both relations present; the host bracket's uncertainty 305 943 ns (`ezsdr.radio.uhd.host_bracket`, drift uncertainty 1e-4).
- Anchor drift over 10.000921 s: −10 630 ticks (−53 µs, about −5.3 ppm device against host).

## B3 — `hw_b3_receive_at_t0`: pass

- The capture starts at receive sample 0, no gap; the first block at index 0, 2 004 ms after `start` (T0 is 2 s ahead).
- `applied`: rate 1 000 000 = claim; **frequency read-back 1 000 000 000 Hz, difference 0.0** (K13 expected < 0.05 Hz); gain 0; antenna RX2.
- ~~**UR-25 finding: the X300 did not honour the timed stop of the continuous stream.**~~ `timing`: `rx_stop` at tick 403 313 197 with its cut at 403 513 197; samples past the cut arrived and uhd-rx stopped the stream untimed (`rx_stop_untimed`) at 404 292 083 (3.9 ms after the cut), as UR-25 provides. The step passes. **Corrected in session 2 (part 2):** no timed stop reached the device in this Run. uhd-rx hands the device a timed stop only when the cut is at least the device lead (2 ms) away (`stop_at`, `rx.rs:246`); an orderly stop's cut is `stop_tail_ns` (1 ms) after it, so the stream was stopped by UR-25's fallback (untimed, on the first sample past the cut) by design. What the X300 does with a timed stop is measured directly in "Session 2, part 2" (it stops at once, whatever the time: UHD 4.10's FPGA does not support a timed STOP).

## B4 — `hw_b4_capture_at_a_sample_index`: pass

The capture's first sample is receive sample 50 000.

## B5 — `hw_b5_overflow`: pass

- No overflow with a 300 ms stall (the socket buffer absorbed it); **an overflow with a 1 000 ms stall**: `radio.RX_OVERFLOW { cause: overrun }` and an `overflow_restart` gap whose `lost` equals its length (the test's assertions).
- `stats`: rx_blocks 7 734, rx_samples 15 465 011, rx_overflows 1, link_drops_seen 1 530, **rx_off_lattice 2 179**, rx_before_origin 0.
- Finding: 2 179 blocks came with a first tick off the stream's 20-tick lattice (10 Msps), which UR-17 rounds to the nearest sample and counts — INFERRED: every block after UHD's own restart of the overrun stream, whose new start UHD chooses. The restart gap's length itself was not printed (the test asserts on it); record it on a rerun (test output may be added).

## B6, B7 (Rust), B8 — session 1 did not run them

Session 1's script ran them from the wrong directory (`cargo` found no `Cargo.toml` in `/bench`): nothing was transmitted by them (the three logs are kept in `~/ezsdr-bench/s1-wrongdir/`). Session 2 runs them with `-w /work`.

## Session 2 (2026-09-30, on `usrp-lnx02` itself)

A Claude Code session on `usrp-lnx02`, from `de7b4b2`. Before transmitting: `uhd_usrp_probe` (`~/ezsdr-bench/probe-s2.log`) shows the bench as session 1 left it — X300, FPGA 39.3 `d375d68`, `OBX TX`/`OBX RX` in slot A, slot B `Unknown (0xffff) - 0`; `enp2s0f0np0` up at `192.168.40.10/24`, MTU 9000; `rmem_max` 50 000 000, `wmem_max` 33 554 432. Every step below runs as handoff.md §4 gives it (`docker run … -w /work … cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware <test> -- --ignored --nocapture > ~/ezsdr-bench/<test>.log 2>&1`).

RF, checked in the code before the first transmission: B6 and B8 transmit at the profile's default 1 GHz (B6's Spec sets `radio.tx.frequency_hz` to the receive frequency, B8's Session inherits `x310_defaults`), transmit gain 0 dB (the default; neither sets one), antenna `TX/RX`, a QPSK PN waveform of amplitude 0.4 (`pn`), into the 30 dB loopback. **B6's and B8's bench profiles carry no `radio.rf_envelope`** (`bench_profile(…, json!({}), …)`; only B7's does), unlike bench.md's "every bench profile carries" it: the transmissions stayed inside bench.md's RF conditions, but nothing but the test code held them there. Recorded, not changed.

### B6 — `hw_b6_txrx_and_repeat`: fail (the test's detection threshold; the burst is heard)

First run, as committed (`de7b4b2`):

```
running 1 test
[INFO] [UHD] linux; GNU C++ version 15.2.0; Boost_109000; UHD_4.10.0.0-0-unknown
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz

thread 'hw_b6_txrx_and_repeat' (21) panicked at crates/ezsdr-radio-uhd/tests/common/mod.rs:468:88:
the burst's correlation peak
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
hw_b6_txrx_and_repeat --- FAILED
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 9 filtered out; finished in 5.54s
```

The capture is gone with the test's `TempDir`, so the second run adds `diagnose` to `tests/common/mod.rs` (test output only: the capture's RMS and the peaks of the complex correlation's magnitude and of its real part, as fractions of the waveform's energy, i.e. the loop's amplitude gain):

```
B6 burst: 5000 samples, rms 0.00496; |corr| peak Some((44, 0.015181948, 24.60064)); Re(corr) peak Some((44, 0.013803906)); threshold 0.01580 (fractions of the energy 320.00427)

thread 'hw_b6_txrx_and_repeat' (81) panicked at crates/ezsdr-radio-uhd/tests/common/mod.rs:498:88:
the burst's correlation peak
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 9 filtered out; finished in 10.56s
```

- **The burst is heard at sample 44** of the capture that starts with it (T0 + 10 000 samples at 1 Msps): the transmit-to-receive delay is 44 samples (MR-3 claims 45, INFERRED).
- **The loop's amplitude gain is 0.0152 (−36.4 dB)** with the OBX at 0 dB transmit and receive gain through the 30 dB attenuator, at a phase of 24.6° (the TX and RX LOs' offset).
- The cause is the test, not the Module: `correlate` accepts a peak only when the **real part** of the correlation exceeds `0.5 × 0.0316` of the energy (−36.0 dB), a threshold that assumes a −30 dB loop (6 dB of margin) and a zero phase. The real loop is 0.4 dB below it in magnitude, and the real part (0.0138) loses another 0.8 dB to the 24.6° phase; any other LO phase moves it further (cos φ). Python's `bench_loopback.py` found its peaks because it takes `np.abs` of the complex correlation.
- Per "If a step fails": fixed in the test code with a test that reproduces it first (next entry), then B6 is run again.

**The fix (test code only; no Module code, no profile value).** `correlate_finds_the_bench_loop_at_any_phase` (`tests/fake.rs`) puts the waveform into 5 000 samples of noise (RMS 0.003) at sample 44 with gain 0.0152 at 24.6°, 90°, 180° and −120°, and noise alone; with the committed `correlate` it failed (`left: None, right: Some(44)` at 24.6°, `~/ezsdr-bench/correlate-before.log`). `correlate` now takes the **magnitude** of the complex correlation and accepts its peak when it stands `CORRELATION_PEAK_OVER_MEDIAN` = 8 times above the correlation's median (the noise floor; noise alone peaks about 3.5 times over a few thousand offsets), not above an absolute loop gain. Then the new test passes.

A second gap found on reading the step: on the bench (`exact = false`) the test checked the repeat's capture only for a correlation peak, not bench.md's "back to back across every wrap" (the sample comparison runs only on the fake). Added: `back_to_back` correlates every whole period after the first peak and the step asserts each has more than half the first's gain and a phase within ±20° of it; `back_to_back_sees_a_slip_in_a_cabled_repeat` checks it sees a 7-sample slip. After both: `cargo test --workspace` 849 passed (847 + the two tests); `--features uhd`: fake 99, `uhd_api` 4, hardware 10 ignored; clippy `-D warnings` clean.

### B6 — `hw_b6_txrx_and_repeat`, rerun: pass

```
running 1 test
[INFO] [UHD] linux; GNU C++ version 15.2.0; Boost_109000; UHD_4.10.0.0-0-unknown
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
B6 burst: 5000 samples, rms 0.00494; correlation peak Some((44, 0.0151171, 25.14115)) (offset, gain, phase °); peak over median 3113.8 (needs > 8)
B6: transmit-to-receive delay 44 samples
B6 repeat: 4000 samples, rms 0.01110; correlation peak Some((44, 0.015422871, 24.600775)) (offset, gain, phase °); peak over median 28.9 (needs > 8)
B6 repeat: 3 whole periods from sample 44 (offset, gain, phase °): [(44, 0.015422871, 24.600775), (1044, 0.015420231, 24.492924), (2044, 0.01541671, 24.440487)]
B6 bursts: [{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":1000,"target":{"domain":{"local":4,"node":0},"ticks":2011816},"wraps":1}] / [{"actual_start":null,"blocks":20,"end":"stop","late_by":null,"requested_target":null,"samples":20000,"target":{"domain":{"local":4,"node":0},"ticks":2002198},"wraps":20}]
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 8.55s
```

- **The transmit-to-receive delay: 44 samples at 1 Msps** (MR-3's 45 is INFERRED; the profile's `tx_path_delay_samples` stays 45, spec 18 §3: an input for Phase 8). The same 44 in all three runs of the session.
- The loop: gain 0.0151–0.0154 (−36.4 to −36.2 dB) at 0 dB transmit and receive gain through 30 dB, phase 24.4°–25.3° (the TX and RX LOs' offset, steady within a run and across the three runs of this power cycle).
- The repeat: back to back across the capture's two wraps (1 044 and 2 044), gain and phase unchanged to 0.2° and 0.03 %; the repeat's record `wraps: 20` (20 000 samples, ended by `stop` when the Run finished after its capture, not a 3 s repeat as bench.md words it); no `TX_UNDERFLOW`.
- No `TIME_ERROR` in either Run's bursts (`late_by: null`).

### B7 (Rust) — `hw_b7_session_loopback`: pass

First run (as `6aee7fc`): `B7 time errors: []`, `test result: ok. 1 passed … finished in 17.56s`. The test prints nothing else, so `println!`s were added (test output only: the refusal, `diagnose` of the capture, the capture's continuity and the Manifest's sample clocks) and it was run twice more; the last run:

```
running 1 test
[INFO] [UHD] linux; GNU C++ version 15.2.0; Boost_109000; UHD_4.10.0.0-0-unknown
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
B7 refused: Rejected { violations: [Violation { check: Namespace("radio.rf_envelope"), key: Some(Key("radio.tx.frequency_hz")), requested: Some(Num(1100000000.0)), reason: "RM-19: radio: 1100000000 Hz is in no allowed band" }] }
B7 capture: 5000 samples, rms 0.01113; correlation peak Some((786, 0.015383206, 24.249798)) (offset, gain, phase °); peak over median 28.9 (needs > 8)
B7 capture asked at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 411349568 }; artifact [ContinuityMap { domain: ClockDomainId { node: NodeId(0), local: 3 }, channels: 1, valid: [[Segment { start: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 54425 }, len: 5000 }]], gaps: [], channel_gaps: [], first: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 54425 }, end: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 59425 } }]
B7 sample clocks: [SampleClockRecord { stream: ResourceId { node: NodeId(0), path: "usrp/rx" }, domain: ClockDomainId { node: NodeId(0), local: 3 }, root: ClockDomainId { node: NodeId(0), local: 2 }, root_ticks_per_tick: Rational { num: 200, den: 1 }, origin: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 400464600 }, ended_at: None, nominal_rate: Rational { num: 1000000, den: 1 } }, SampleClockRecord { stream: ResourceId { node: NodeId(0), path: "usrp/tx" }, domain: ClockDomainId { node: NodeId(0), local: 4 }, root: ClockDomainId { node: NodeId(0), local: 2 }, root_ticks_per_tick: Rational { num: 200, den: 1 }, origin: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 401063800 }, ended_at: None, nominal_rate: Rational { num: 1000000, den: 1 } }]
B7 time errors: []
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 5.56s
```

- **No `TIME_ERROR`** in all three runs (spike K6; on the bench the test demands none).
- **The retune outside the envelope is refused** by `radio.rf_envelope` naming `radio.tx.frequency_hz` = 1.1 GHz (§58 #16).
- **The capture starts at its instant:** asked at root tick 411 349 568 = receive sample (411 349 568 − 400 464 600) / 200 = 54 424.84; the artifact's first sample is 54 425, the next sample on the lattice (32 root ticks, 160 ns, after the instant), no gap, 5 000 samples.
- The repeat is heard (untimed, so at an arbitrary offset: 2 883 and 786 in the last two runs) at gain 0.0154 and phase 24.2°, the same loop as B6.

### B8 — `hw_b8_leads`: ran (a measurement); the device lead is not reached

What the test implements of bench.md's B8: a Session per lead enables one transmit channel (1 Msps, 1 GHz, 0 dB: the defaults), then `radio.send` of a 100-sample PN burst `at` the Run's `now()` plus the lead (10, 5, 3, 2, 1.5, 1, 0.5 ms), waits 100 ms and prints the burst's `TIME_ERROR`s and the `timing` section. Run 1, as committed:

```
running 1 test
[INFO] [UHD] linux; GNU C++ version 15.2.0; Boost_109000; UHD_4.10.0.0-0-unknown
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
B8 lead 10000 µs: TIME_ERROR []
B8 lead 10000 µs: timing [{"at":343073,"start_up_until":400343073,"what":"arm"},{"lead_ns":1999997345,"t0":400525800,"what":"start"},{"channels":1,"dir":"tx","origin":401266400,"what":"enabled"},{"host_delay_ms":2004,"index":0,"what":"first_rx_block"},{"at":421720943,"until":421920943,"what":"rx_stop"},{"at":422477227,"cut":421920943,"what":"rx_stop_untimed"},{"at":421720943,"done":442820017,"mode":"Orderly","what":"stop"}]
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
B8 lead 5000 µs: TIME_ERROR []
B8 lead 5000 µs: timing [{"at":358079,"start_up_until":400358079,"what":"arm"},{"lead_ns":1999997235,"t0":400601800,"what":"start"},{"channels":1,"dir":"tx","origin":401362400,"what":"enabled"},{"host_delay_ms":2004,"index":0,"what":"first_rx_block"},{"at":421698010,"until":421898010,"what":"rx_stop"},{"at":422568528,"cut":421898010,"what":"rx_stop_untimed"},{"at":421698010,"done":442786220,"mode":"Orderly","what":"stop"}]
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
B8 lead 3000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(35000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(3077)}}]
B8 lead 3000 µs: timing [{"at":310390,"start_up_until":400310390,"what":"arm"},{"lead_ns":1999998390,"t0":400521000,"what":"start"},{"host_delay_ms":2004,"index":0,"what":"first_rx_block"},{"channels":1,"dir":"tx","origin":401428200,"what":"enabled"},{"at":421767631,"until":421967631,"what":"rx_stop"},{"at":422515799,"cut":421967631,"what":"rx_stop_untimed"},{"at":421767631,"done":442801364,"mode":"Orderly","what":"stop"}]
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
B8 lead 2000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(1197000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(2022)}}]
B8 lead 2000 µs: timing [{"at":347901,"start_up_until":400347901,"what":"arm"},{"lead_ns":1999997170,"t0":400566400,"what":"start"},{"channels":1,"dir":"tx","origin":401042000,"what":"enabled"},{"host_delay_ms":2004,"index":0,"what":"first_rx_block"},{"at":421464089,"until":421664089,"what":"rx_stop"},{"at":422136724,"cut":421664089,"what":"rx_stop_untimed"},{"at":421464089,"done":442570291,"mode":"Orderly","what":"stop"}]
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
B8 lead 1500 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(1644000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(1643)}}]
B8 lead 1500 µs: timing [{"at":373295,"start_up_until":400373295,"what":"arm"},{"lead_ns":1999997600,"t0":400509800,"what":"start"},{"channels":1,"dir":"tx","origin":401091800,"what":"enabled"},{"host_delay_ms":2003,"index":0,"what":"first_rx_block"},{"at":421558443,"until":421758443,"what":"rx_stop"},{"at":422479452,"cut":421758443,"what":"rx_stop_untimed"},{"at":421558443,"done":442492684,"mode":"Orderly","what":"stop"}]
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
B8 lead 1000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(1992000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(1173)}}]
B8 lead 1000 µs: timing [{"at":314803,"start_up_until":400314803,"what":"arm"},{"lead_ns":1999996920,"t0":400432800,"what":"start"},{"channels":1,"dir":"tx","origin":401030400,"what":"enabled"},{"host_delay_ms":2004,"index":0,"what":"first_rx_block"},{"at":421386302,"until":421586302,"what":"rx_stop"},{"at":421985735,"cut":421586302,"what":"rx_stop_untimed"},{"at":421386302,"done":442380253,"mode":"Orderly","what":"stop"}]
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
B8 lead 500 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(2652000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(515)}}]
B8 lead 500 µs: timing [{"at":394549,"start_up_until":400394549,"what":"arm"},{"lead_ns":1999997285,"t0":400597400,"what":"start"},{"channels":1,"dir":"tx","origin":401341200,"what":"enabled"},{"host_delay_ms":2003,"index":0,"what":"first_rx_block"},{"at":421663947,"until":421863947,"what":"rx_stop"},{"at":422554451,"cut":421863947,"what":"rx_stop_untimed"},{"at":421663947,"done":442687382,"mode":"Orderly","what":"stop"}]
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 51.90s
```

Run 2 adds the `bursts` section to the output (`hardware.rs`, test output only); its `TIME_ERROR` and `bursts` lines:

```
B8 lead 10000 µs: TIME_ERROR []
B8 lead 10000 µs: bursts [{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":100,"target":{"domain":{"local":4,"node":0},"ticks":10020},"wraps":1}]
B8 lead 5000 µs: TIME_ERROR []
B8 lead 5000 µs: bursts [{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":100,"target":{"domain":{"local":4,"node":0},"ticks":5030},"wraps":1}]
B8 lead 3000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(189000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(3088)}}]
B8 lead 3000 µs: bursts []
B8 lead 2000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(1154000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(2085)}}]
B8 lead 2000 µs: bursts []
B8 lead 1500 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(1995000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(1556)}}]
B8 lead 1500 µs: bursts []
B8 lead 1000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(2085000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(1166)}}]
B8 lead 1000 µs: bursts []
B8 lead 500 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(2690000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(518)}}]
B8 lead 500 µs: bursts []
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 63.91s
```

- **At 10 and 5 ms the burst is sent**: a record ending `eob`, 100 samples, `late_by: null`, and no `TIME_ERROR` (no `late_at_device` from the device).
- **At 3 ms and less the Module drops the burst before the device sees it**: `TIME_ERROR { cause: late, outcome: drop }` and no burst record, in both runs. The drop is UR-21's: uhd-control decides `LatePolicy::decide(clocks, at, now', 2 ms)` when it books the Action, `now'` its receipt, and the Session's `radio.send` has the policy `drop`. From `late_by = (now' + 2 ms) − at` with `at = now() + lead`, **receipt − submission = late_by + lead − 2 ms: 1 035, 1 197, 1 144, 992, 1 152 µs (run 1) and 1 189, 1 154, 1 495, 1 085, 1 190 µs (run 2)** — within the delivery allowance of 3 ms (UR-14), the poll period 1 ms plus the booking.
- **So the device lead (the least lead with no `late_at_device`) is not measured**: the Module's own 2 ms device lead refuses every burst that would probe it. What is measured is that a 5 ms lead from submission (the declared `min_timed_command_lead`, 2 + 3 ms) is enough and 3 ms is not, and that the device accepted a burst whose target was about 5 − 1.2 = 3.8 ms after its receipt. Measuring the device lead needs a test below UR-21 (raw `Device::tx_send` with a time spec at leads under 2 ms): not implemented; recorded for Phase 8 (spec 18 §3's 2 ms stays).
- ~~**UR-25 again: the timed receive stop is never honoured.**~~ In all 7 Runs of run 1 the orderly stop's cut was 1.000 ms after the stop and uhd-rx stopped the stream untimed when samples past the cut arrived, 2.78, 3.35, 2.74, 2.36, 3.61, 2.00, 3.45 ms after the cut (B3's was 3.9 ms): UR-25's fallback, 7 of 7. **Corrected in session 2 (part 2):** no timed stop reached the device in this Run. uhd-rx hands the device a timed stop only when the cut is at least the device lead (2 ms) away (`stop_at`, `rx.rs:246`); an orderly stop's cut is `stop_tail_ns` (1 ms) after it, so the stream was stopped by UR-25's fallback (untimed, on the first sample past the cut) by design. What the X300 does with a timed stop is measured directly in "Session 2, part 2" (it stops at once, whatever the time: UHD 4.10's FPGA does not support a timed STOP).
- The transmit SampleClock's origin (`enabled`) is 2.4–4.5 ms after T0 in the 7 Runs.
- **Not implemented, so not measured** (bench.md B8's other rows, handoff.md §4's B8 row): timed retunes at the leads; `cold` restart leads 100/50/25/10 ms; the transmit end after `Stop`; in-flight windows 10/5/3/2 ms against `TX_UNDERFLOW`; the preemption bound at 20/10/5 ms; the queue depth of timed OBX tunes (UR-24); a dedicated timed-receive-stop measurement (answered above from the `timing` of every Run instead). Spec 18 §3's restart lead 50 ms, in-flight window 10 ms, release window 3 ms and queue depth 16 stay INFERRED.

### B5 — `hw_b5_overflow`, rerun for the restart gap: pass

Session 1 did not print the gap. `hw_b5_overflow` now prints (test output only) the capture's gaps by cause, the `overflow_restart` gap, the decoded `RX_OVERFLOW` payload, the `timing` section and the sample clocks. Run 2 (without `timing`):

```
B5 no overflow with a 300 ms stall; the socket buffer absorbed it
OB5 overflowed with a 1000 ms stall: {"link_drops_seen":1375,"rx_before_origin":0,"rx_blocks":7729,"rx_errors":0,"rx_off_lattice":2174,"rx_overflows":1,"rx_overlapping":0,"rx_samples":15455793,"tx_bursts":0,"tx_samples":0}
B5 1266 gaps: 1265 link drops of 2750000 samples in all, from sample Some(5180000) to Some(10980000)
B5 gap OverflowRestart at receive sample 11108408 for 4562577 samples (456.2577 ms at 10 Msps), lost Some(4562577)
B5 capture 0 … 20018370, 1267 valid segment(s)
B5 RX_OVERFLOW: Ok(RxOverflowPayload { cause: Overrun, lost: 4562577, restart_gap_ns: 456257700 })
```

Run 3, whole output:

```
running 1 test
[INFO] [UHD] linux; GNU C++ version 15.2.0; Boost_109000; UHD_4.10.0.0-0-unknown
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
thread 'hw_b5_overflow' (81) panicked at crates/ezsdr-radio-uhd/tests/common/mod.rs:496:5:
no overflow; the host's socket buffer absorbed the stall
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
B5 no overflow with a 300 ms stall; the socket buffer absorbed it
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
OB5 overflowed with a 1000 ms stall: {"link_drops_seen":1301,"rx_before_origin":0,"rx_blocks":7729,"rx_errors":0,"rx_off_lattice":2174,"rx_overflows":1,"rx_overlapping":0,"rx_samples":15455088,"tx_bursts":0,"tx_samples":0}
B5 1218 gaps: 1217 link drops of 2602000 samples in all, from sample Some(5382000) to Some(10982000)
B5 gap OverflowRestart at receive sample 11108408 for 4565115 samples (456.5115 ms at 10 Msps), lost Some(4565115)
B5 capture 0 … 20020203, 1219 valid segment(s)
B5 timing [{"at":333313,"start_up_until":400333313,"what":"arm"},{"lead_ns":1999993160,"t0":400572120,"what":"start"},{"host_delay_ms":2000,"index":0,"what":"first_rx_block"},{"ms":1000,"what":"rx_stall"},{"at":800776171,"until":800976171,"what":"rx_stop"},{"at":801017541,"cut":800976171,"what":"rx_stop_untimed"},{"at":800776171,"done":821439602,"mode":"Orderly","what":"stop"}]
B5 sample clocks: [SampleClockRecord { stream: ResourceId { node: NodeId(0), path: "usrp/rx" }, domain: ClockDomainId { node: NodeId(0), local: 3 }, root: ClockDomainId { node: NodeId(0), local: 2 }, root_ticks_per_tick: Rational { num: 20, den: 1 }, origin: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 400572120 }, ended_at: None, nominal_rate: Rational { num: 10000000, den: 1 } }]
B5 RX_OVERFLOW: Ok(RxOverflowPayload { cause: Overrun, lost: 4565115, restart_gap_ns: 456511500 })
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 18.13s
```

- **The `overflow_restart` gap: 4 562 577 and 4 565 115 samples at 10 Msps = 456.3 and 456.5 ms** (`RX_OVERFLOW.restart_gap_ns` 456 257 700 and 456 511 500; `lost` = the gap's length, as the test asserts), both starting at receive sample **11 108 408** (1 110.8 ms into the stream).
- What the gap is made of (from the numbers; the split is INFERRED): the stall (`with_rx_stall(500 ms, 1 000 ms)`) stops uhd-rx's reads from 500 ms after the first block (host delay 2 000 ms after `start`, i.e. about receive sample 0) to about 1 500 ms. The socket buffer absorbed 1 110.8 − 500 = 610.8 ms of stream (24.4 MB of `sc16` at 10 Msps, with `rmem_max` 50 000 000), then the device overran and stopped. The gap ends at receive sample 15 673 523 (1 567.4 ms), **about 67 ms after the stall ended**; in those 67 ms uhd-rx read the 610 ms backlog (the host link, `drop_oldest` capacity 64, dropped 1 217–1 265 blocks of it: the `link_drop` gaps between samples 5.2 M and 11.0 M) and UHD restarted the stream. So the gap an overflow leaves depends on how long the reader was away, and **UHD's own restart after it detects the overrun is at most ~67 ms** here, which does not contradict MR-3's 50 ms (profile `overflow_restart_gap_ns` 50 ms, unchanged: an input for Phase 8, which should measure the restart from the host's detection of the overflow, a time the Module does not record).
- The stall that first overflowed: 1 000 ms (300 ms did not, in all three runs of the two sessions); `rmem_max` 50 000 000, `wmem_max` 33 554 432.
- `rx_off_lattice` 2 174 (session 1: 2 179): again every block after the restart is off the 20-tick lattice (INFERRED, session 1's finding (b)).
- The orderly stop at the end of the Run: `rx_stop_untimed` 0.21 ms after the cut (UR-25's fallback; no timed stop was issued, see the correction under B3).

## B7 (Python) — ran; one finding

`EZSDR_SERVER=/cargo-target/release/ezsdr-server EZSDR_PROFILE=/bench/bench-session.json` (bench.md's Session profile with `x310-obx`, `addr=192.168.40.36`, the capture directory `/bench/b7-captures`):

- `minimal.py` (Vision §3's snippet, unchanged): `captured 100000 samples, mean power 0.0001` (the 0.5-amplitude tone through 30 dB), 3 logged calls; Manifest `~/ezsdr-bench/ezsdr-runs/session-7-0/manifest.json`.
- `bench_loopback.py`: the rate 19.5 Msps coerced to 20 Msps (a new SampleClock); **TIME_ERROR events: 0** (spike K6); y's correlation peak at sample 7 801, z's at 6 773; `rec_0` first sample tick 54 543, `rec_1` 179 571 (receive clock); asked y at root tick 411 438 339, z at 436 443 948; Manifest `~/ezsdr-bench/ezsdr-runs/session-7-1/manifest.json`.
- **Finding: "the retune outside the RF envelope was admitted".** Not a Module defect: the example retunes `sdr.rx.frequency`, and RM-19 limits only transmit frequencies ("Receive frequencies are not limited: the envelope is about emission", design/07). `hw_b7_session_loopback` retunes `radio.tx.frequency_hz`, which is the check bench.md asks for. Fix the example (`sdr.tx.frequency = 2.4e9`), then run it again.
- Whether z started at t (EA-17): derived in session 2 from `session-7-1`'s Manifest, below.

### B7 (Python), session 2 rerun with the fixed example (`7221612`): pass

Same container and environment as session 1 (`-w /bench -e PYTHONDONTWRITEBYTECODE=1 -e PYTHONPATH=/work/python -e EZSDR_SERVER=/cargo-target/release/ezsdr-server -e EZSDR_PROFILE=/bench/bench-session.json`; the release server rebuilt from `7221612`, the server code unchanged since session 1). `bench_loopback.py` now retunes `sdr.tx.frequency = 2.4e9` (the fix: session 1's finding (c); the v58 scan and the Python suite, 23 tests against the debug server, pass). Session 1's two logs are kept as `~/ezsdr-bench/b7_minimal.s1.log` and `b7_loopback.s1.log`.

`python3 /work/python/examples/minimal.py`:

```
{mn}
```

`python3 /work/python/examples/bench_loopback.py`:

```
{lp}
```

- **The transmit retune outside the envelope raises `ezsdr.Rejected` naming `radio.rf_envelope`** and is logged: `session-7-3`'s `action_log` seq 5, `set_parameter radio.tx.frequency_hz 2400000000.0`, outcome `rejected`, check `radio.rf_envelope`, `RM-19: radio: 2400000000 Hz is in no allowed band` (§58 #16).
- No `TIME_ERROR` (all three sources' counters 0); the rate 19.5 Msps coerced to 20 Msps with a new receive SampleClock (local 5, 20 Msps, from root tick 461 040 600, the 1 Msps clock ended at 451 040 600: a 50 ms restart, UR-25's restart lead).
- `y` and `z` hold the waveform: from the capture files (`~/ezsdr-bench/b7-captures/`), the complex correlation with `x` peaks at gain 0.0149 (session 2) / 0.0153 (session 1), phase 19–20°, 30.7 times its median, and again one period (1 000 samples) later at the same gain.

**EA-17: `z` starts at `t`.** The receive SampleClock (local 3, 1 Msps, 200 root ticks a sample) has its origin at root tick 400 529 800 (`session-7-1`) and 400 476 200 (`session-7-3`). Each capture asked at root tick `a` starts at the first receive sample at or after `a`:

| Session | capture | asked (root tick) | = receive sample | first sample | after the instant |
|---|---|---|---|---|---|
| `session-7-1` | `y` (`after(0.05)`) | 411 438 339 | 54 542.69 | 54 543 | 305 ns |
| `session-7-1` | `z` (`at=t`, `t = sleep(0.1)`) | 436 443 948 | 179 570.74 | 179 571 | 260 ns |
| `session-7-3` | `y` | 411 283 596 | 54 036.98 | 54 037 | 20 ns |
| `session-7-3` | `z` | 436 281 252 | 179 025.26 | 179 026 | 740 ns |

Every capture is 20 000 samples with no gap. So a capture requested at an instant the Run has just reached (`t` returned by `sleep`) still starts there: the local request does not outrun the device's receive latency on this bench (the answer to EA-17's INFERRED case). Why — the samples at `t` not yet delivered to the recorder when the request reached it, or the recorder still holding them — is not visible in the Manifest (INFERRED either way).

## Still to do

1. ~~Rerun B6, B7 (Rust) and B8 from `/work`.~~ Done in session 2 (B6 after a fix of the test's correlation).
2. ~~Fix `python/examples/bench_loopback.py` to retune the transmitter; rerun B7 (Python); derive EA-17's answer from the Manifests.~~ Done in session 2.
3. B9: the unplug (manual: the owner pulls the 10 GbE cable during a receive Run, and during a transmit-only Session — the latter has no test yet), the rerun of B3–B7 with no false `DEVICE_LOST`; the USRP2 probe if a USRP2 is at hand.
4. Fill the table at the end of `plan/spikes/2026-09-26-uhd.md`; update handoff.md.
5. ~~B8's unimplemented rows~~ Measured in session 2 part 2 (below).
6. design-notes §11 F1–F3: the owner's decision, then the fixes (each with a `FakeDevice` test first) and B3–B8 again.

## Session 2 in short

B6 pass (after the test's correlation fix), B7 Rust pass, B7 Python pass (fixed example), B8 run, B5's restart gap recorded. Numbers for Phase 8 (spec 18 §3 and the profiles unchanged): transmit-to-receive delay **44 samples** at 1 Msps (profile 45); loop gain −36.3 dB through 30 dB at 0 dB gains; a 5 ms lead from submission sends, 3 ms is dropped by UR-21 (receipt 1.0–1.5 ms after submission), the device lead itself not reached; the orderly receive stops all by UR-25's untimed fallback, 0.2–3.9 ms after the cut (no timed stop was issued in those Runs: corrected in part 2, which measures the timed stop itself); the overflow gap 456 ms after a 1 s stall, UHD's restart ≤ ~67 ms after the reader returns; EA-17: a capture at the `t` `sleep` returned starts at `t`. Test code changed (no Module code): `correlate` phase-blind with a noise-floor threshold, `back_to_back`, and `println!`s in B5, B7, B8 (`6aee7fc` … `226122d`). The bench profiles of B6 and B8 carry no `radio.rf_envelope`.

## Session 2, part 2 — what the bench could still answer with the owner away (2026-09-30)

The owner, away from the bench: "ちょっと今手元にUSRPがなくて遠隔でやってます．なので，とりあえず今のうちに今の状態でUSRPを使って確認しておいた方がいいことを考えて全部やってください". Nothing that needs a hand at the bench (B9's unplug) was done. What was: bench.md B8's rows the Module's own rules keep a Session from probing, measured on the `Device` itself (`hw_b8_raw_*`, test code in `hardware.rs`: the device used as the Module drives it); the rest of B8 through the Module; beyond bench.md, the receive rates, the loop delay by rate and B9's other half (long Runs with no false `DEVICE_LOST`); then B1–B8 once more at the commit of those tests (`85ac77b`). Every log is `~/ezsdr-bench/<test>.log` (the final B1–B8 in `~/ezsdr-bench/s2-final/`). Below each test's output keeps its `B…` lines; a run of `TimeError` reports is shortened to its count and UHD's printed `L`s (one per late packet) to theirs.

RF for the raw tests (`raw_tx` refuses anything else before it sends): the transmitter at 999.5–1 000.5 MHz only, at most 2 Msps (so the emission stays inside the bench envelope's 999–1 001 MHz), 0 dB, antenna `TX/RX`, amplitude ≤ 0.4, into the 30 dB loopback; the receiver's retunes stayed inside 999.6–1 000.4 MHz too. The raw tests bypass the Module's RF envelope, so these limits are the only guard there. The Module-level steps that transmit (B6, B8, and the new ones) now carry the bench envelope in their profile (`bench_envelope`, in `tests/common/mod.rs`: session 2's finding that B6's and B8's did not).

UHD 4.10's source, in the Docker image under `/root/tmp/uhd-4.10.0.0/`, was read for the receive stop and the overrun restart (VERIFIED below means read there and seen on the bench).

### The receive stop's time — `hw_b8_raw_rx_timed_stop`: **the X300 ignores it**

A continuous 1 Msps stream, then `rx_stop(Some(now + lead))` (lead 0: untimed); where does the last sample end?

```
B8 rx stop lead 0 ms: first block Some((24256800, 2000)) (start 24256800); stop issued at 64615084…64698267 for now; last sample end 64689800: -0.042 ms from the issue, +0.374 ms from the stop's time; after the stop: 2 blocks, []
B8 rx stop lead 1 ms: first block Some((108960000, 2000)) (start 108960000); stop issued at 149320947…149401582 for 149520947; last sample end 149398400: -0.016 ms from the issue, -0.613 ms from the stop's time; after the stop: 2 blocks, []
B8 rx stop lead 5 ms: first block Some((193682600, 2000)) (start 193682600); stop issued at 234042826…234102171 for 235042826; last sample end 234097800: -0.022 ms from the issue, -4.725 ms from the stop's time; after the stop: 2 blocks, []
B8 rx stop lead 20 ms: first block Some((278382800, 2000)) (start 278382800); stop issued at 318741398…318804796 for 322741398; last sample end 318802800: -0.010 ms from the issue, -19.693 ms from the stop's time; after the stop: 2 blocks, []
B8 rx stop lead 100 ms: first block Some((363106000, 2000)) (start 363106000); stop issued at 403458422…403541707 for 423458422; last sample end 403538400: -0.017 ms from the issue, -99.600 ms from the stop's time; after the stop: 2 blocks, []
B8 rx stop lead 500 ms: first block Some((447814800, 2000)) (start 447814800); stop issued at 488172567…488255353 for 588172567; last sample end 488248400: -0.035 ms from the issue, -499.621 ms from the stop's time; after the stop: 2 blocks, []
```

- **At every lead from 1 ms to 500 ms the stream ends where the stop was issued** (−0.010 to −0.042 ms from the issue), up to 499.6 ms before the stop's time. VERIFIED from the FPGA source: `fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v` puts every command into the command FIFO "except STOP" (line 171) and says at line 526 "Nothing to do but stop (timed STOP commands are not supported)"; the host sends the time (`radio_control_impl.cpp:1116–1131`), the FPGA drops it.
- **This answers UR-25's INFERRED point in the negative**, and it matters to the Module (design-notes §11 F1): uhd-rx hands the device a timed stop for a `cold` change's `e1` up to the restart lead (50 ms) early (`release_stop`, `rx.rs:213–225`), so the stream stops that much before `e1`. See the next test.
- The earlier "timed stop not honoured" records of B3 and B8 were not about this: no timed stop was issued in those Runs (corrected above).

### A capture across a `cold` receive change — `hw_b8_cold_change_capture`: **50 ms lost without a flag, then a late restart**

A Session receiving at 1 Msps; a capture of 200 000 samples asked 20 ms ahead; 60 ms later `radio.rx.sample_rate_hz = 2e6`:

```
B8 cold: capture asked at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 424645731 }; rate change submitted at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 432766941 }: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }
LB8 cold: termination Stopped { cause: Client }
B8 cold: sample clocks [SampleClockRecord { stream: ResourceId { node: NodeId(0), path: "usrp/rx" }, domain: ClockDomainId { node: NodeId(0), local: 3 }, root: ClockDomainId { node: NodeId(0), local: 2 }, root_ticks_per_tick: Rational { num: 200, den: 1 }, origin: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 400562800 }, ended_at: Some(TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 442887800 }), nominal_rate: Rational { num: 1000000, den: 1 } }, SampleClockRecord { stream: ResourceId { node: NodeId(0), path: "usrp/rx" }, domain: ClockDomainId { node: NodeId(0), local: 4 }, root: ClockDomainId { node: NodeId(0), local: 2 }, root_ticks_per_tick: Rational { num: 100, den: 1 }, origin: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 452887800 }, ended_at: None, nominal_rate: Rational { num: 2000000, den: 1 } }]
B8 cold: timing [{"at":426458,"start_up_until":400426458,"what":"arm"},{"lead_ns":1999996500,"t0":400562800,"what":"start"},{"host_delay_ms":2003,"index":0,"what":"first_rx_block"},{"booked_at":432887797,"e1":442887800,"e2":452887800,"key":"radio.rx.sample_rate_hz","what":"cold_change"},{"at":453003085,"e1":442887800,"e2":452887800,"what":"rx_switch"},{"at":483049900,"requested":452887800,"what":"rx_restart"},{"at":559740334,"until":559940334,"what":"rx_stop"},{"at":560068537,"cut":559940334,"what":"rx_stop_untimed"},{"at":559740334,"done":580241090,"mode":"Orderly","what":"stop"}]
B8 cold: applied [{"at":{"domain":{"local":2,"node":0},"ticks":350628},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.rx.sample_rate_hz","read_back":1000000.0},{"at":{"domain":{"local":2,"node":0},"ticks":350628},"channel":0,"claimed":1000000000.0,"difference":0.0,"key":"radio.rx.frequency_hz","read_back":1000000000.0},{"at":{"domain":{"local":2,"node":0},"ticks":350628},"channel":0,"claimed":0.0,"difference":0.0,"key":"radio.rx.gain_db","read_back":0.0},{"at":{"domain":{"local":2,"node":0},"ticks":350628},"channel":0,"claimed":"RX2","key":"radio.rx.antenna"},{"at":{"domain":{"local":2,"node":0},"ticks":442887800},"issued":{"domain":{"local":2,"node":0},"ticks":432894229},"key":"rx_stop"},{"at":{"domain":{"local":2,"node":0},"ticks":452982620},"channel":0,"claimed":2000000.0,"difference":0.0,"key":"radio.rx.sample_rate_hz","read_back":2000000.0},{"at":{"domain":{"local":2,"node":0},"ticks":452982620},"channel":0,"claimed":1000000000.0,"difference":0.0,"key":"radio.rx.frequency_hz","read_back":1000000000.0},{"at":{"domain":{"local":2,"node":0},"ticks":452982620},"channel":0,"claimed":0.0,"difference":0.0,"key":"radio.rx.gain_db","read_back":0.0},{"at":{"domain":{"local":2,"node":0},"ticks":452982620},"channel":0,"claimed":"RX2","key":"radio.rx.antenna"}]
B8 cold: stats {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":466,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":930722,"tx_bursts":0,"tx_samples":0}
B8 cold: artifact rec_0 continuity [ContinuityMap { domain: ClockDomainId { node: NodeId(0), local: 3 }, channels: 1, valid: [[Segment { start: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 120415 }, len: 41402 }]], gaps: [], channel_gaps: [], first: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 120415 }, end: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 161817 } }, ContinuityMap { domain: ClockDomainId { node: NodeId(0), local: 4 }, channels: 1, valid: [[Segment { start: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 301621 }, len: 158598 }]], gaps: [], channel_gaps: [], first: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 301621 }, end: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 460219 } }]
B8 cold: event radio.LATE_COMMAND from ResourceId { node: NodeId(0), path: "usrp/rx" } at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 473050499 }: {"applied":{"domain":{"local":2,"node":0},"ticks":483049900},"key":null,"requested":{"domain":{"local":2,"node":0},"ticks":452887800}}
B8 cold: event sink.CAPTURE_WRITTEN from ResourceId { node: NodeId(0), path: "sink/rec" } at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 499487139 }: {"artifact":{"continuity":[{"channel_gaps":[],"channels":1,"domain":{"local":3,"node":0},"end":{"domain":{"local":3,"node":0},"ticks":161817},"first":{"domain":{"local":3,"node":0},"ticks":120415},"gaps":[],"valid":[[{"len":41402,"start":{"domain":{"local":3,"node":0},"ticks":120415}}]]},{"channel_gaps":[],"channels":1,"domain":{"local":4,"node":0},"end":{"domain":{"local":4,"node":0},"ticks":460219},"first":{"domain":{"local":4,"node":0},"ticks":301621},"gaps":[],"valid":[[{"len":158598,"start":{"domain":{"local":4,"node":0},"ticks":301621}}]]}],"hash":"sha256:f9f631cf76f16bdab26b99b3f81294332138021189171a7769cddccad4a3570e","id":"rec_0","kind":"sink.capture","marks":[],"partial":false,"size_bytes":1600000,"uri":"file:///tmp/ezsdr-uhd-80-0/local_50_18da120da170283a-0_rec_0.sigmf-data"},"request":0}
```

- The change was booked at 432 887 797 with `e1` 442 887 800 (the old clock's `ended_at`) and `e2` 452 887 800 (the new clock's origin), the restart lead 50 ms each way. uhd-rx handed the device the timed stop for `e1` at 432 894 229 (`applied`, `rx_stop`, 50 ms early), and the device stopped there.
- **The old clock's part of the capture ends at receive sample 161 817 (root 432 926 200), not at `e1` (sample 211 625): 49 808 samples (49.8 ms) that the Stream Contract says exist are missing, with no gap, no flag and no event.** The continuity map is truthful about what it holds (`valid` ends at 161 817) but nothing says the stream ended early. `rx_samples` 930 722 over the Run.
- **Then the restart was late:** the switch (`rx_switch`) ran at 453 003 085, 0.58 ms after `e2`, so the timed start at `e2` came back `LATE_COMMAND` and UR-17 restarted at 483 049 900: the new clock's samples begin at its sample 301 621, 150.8 ms after `e2`. Why (INFERRED from the timing, consistent to the tick): after the early stop no samples come, and uhd-rx ends the old stream only on a sample past the cut or on an `rx_recv` timeout (`RECV_TIMEOUT` 100 ms, `rx.rs:85`) with `now ≥ cut`; the first such timeout came ~100 ms after the stop, i.e. at `e2`. With a device that did stop at `e1` the same wait would still end ~50 ms after `e2`.
- The same early stop happened in B7 (Python) at `sdr.rx.sample_rate = 19.5e6`: `applied` `rx_stop` issued at 441 233 380 for 451 149 800 (`session-7-1`), 441 220 199 for 451 040 600 (`session-7-3`), 49.6 ms early; the Session ended before the switch, and no capture spanned it.
- Recorded as design-notes §11 F1–F2, for the owner; no Module code changed.

### The restart lead — `hw_b8_raw_restart_lead`

A stop (untimed), optionally a rate change (`cold`) and a new streamer, then a timed start `lead` ahead. Run 1's criterion counted the old stream's tail as new blocks (kept as `hw_b8_raw_restart_lead.run1.log`); the final run, with a new streamer added:

```
B8 restart warm lead 100 ms: stop→start issued 0.462 ms (apply 0.000 ms); first new block Some(70851400) for 70851400 (+0 ticks); 2 old blocks drained; other []
B8 restart warm lead 50 ms: stop→start issued 0.482 ms (apply 0.000 ms); first new block Some(122104400) for 122104400 (+0 ticks); 2 old blocks drained; other []
B8 restart warm lead 25 ms: stop→start issued 0.152 ms (apply 0.000 ms); first new block Some(168290800) for 168290800 (+0 ticks); 1 old blocks drained; other []
B8 restart warm lead 10 ms: stop→start issued 0.364 ms (apply 0.000 ms); first new block Some(211521400) for 211521400 (+0 ticks); 2 old blocks drained; other []
B8 restart warm lead 5 ms: stop→start issued 0.344 ms (apply 0.000 ms); first new block Some(253745600) for 253745600 (+0 ticks); 2 old blocks drained; other []
B8 restart warm lead 2 ms: stop→start issued 0.421 ms (apply 0.000 ms); first new block Some(295379000) for 295379000 (+0 ticks); 2 old blocks drained; other []
B8 restart warm lead 1 ms: stop→start issued 0.372 ms (apply 0.000 ms); first new block Some(336809000) for 336809000 (+0 ticks); 2 old blocks drained; other []
B8 restart cold (rate change) lead 100 ms: stop→start issued 0.539 ms (apply 0.119 ms); first new block Some(398075400) for 398075400 (+0 ticks); 2 old blocks drained; other []
B8 restart cold (rate change) lead 50 ms: stop→start issued 0.407 ms (apply 0.063 ms); first new block Some(448685400) for 448685400 (+0 ticks); 1 old blocks drained; other []
B8 restart cold (rate change) lead 25 ms: stop→start issued 0.423 ms (apply 0.072 ms); first new block Some(494905800) for 494905800 (+0 ticks); 2 old blocks drained; other []
B8 restart cold (rate change) lead 10 ms: stop→start issued 0.407 ms (apply 0.058 ms); first new block Some(535587000) for 537523000 (-1936000 ticks); 1 old blocks drained; other []
B8 restart cold (rate change) lead 5 ms: stop→start issued 0.515 ms (apply 0.058 ms); first new block Some(576982200) for 576982200 (+0 ticks); 2 old blocks drained; other []
B8 restart cold (rate change) lead 2 ms: stop→start issued 0.437 ms (apply 0.041 ms); first new block Some(617663400) for 618005400 (-342000 ticks); 1 old blocks drained; other []
B8 restart cold (rate change) lead 1 ms: stop→start issued 0.302 ms (apply 0.038 ms); first new block Some(658203800) for 658203800 (+0 ticks); 1 old blocks drained; other []
B8 restart: rx_open took 0.993 ms
B8 restart cold (rate change, new streamer) lead 100 ms: stop→start issued 202.568 ms (apply 0.067 ms); first new block Some(759249000) for 759249000 (+0 ticks); 0 old blocks drained; other []
B8 restart: rx_open took 0.739 ms
B8 restart cold (rate change, new streamer) lead 50 ms: stop→start issued 202.776 ms (apply 0.089 ms); first new block Some(850968000) for 850968000 (+0 ticks); 0 old blocks drained; other []
B8 restart: rx_open took 0.853 ms
B8 restart cold (rate change, new streamer) lead 25 ms: stop→start issued 202.530 ms (apply 0.077 ms); first new block Some(937010400) for 937010400 (+0 ticks); 0 old blocks drained; other []
B8 restart: rx_open took 0.922 ms
B8 restart cold (rate change, new streamer) lead 10 ms: stop→start issued 202.720 ms (apply 0.073 ms); first new block Some(1020731800) for 1020731800 (+0 ticks); 0 old blocks drained; other []
B8 restart: rx_open took 0.722 ms
B8 restart cold (rate change, new streamer) lead 5 ms: stop→start issued 202.244 ms (apply 0.055 ms); first new block Some(1102712000) for 1102712000 (+0 ticks); 0 old blocks drained; other []
B8 restart: rx_open took 0.668 ms
B8 restart cold (rate change, new streamer) lead 2 ms: stop→start issued 202.439 ms (apply 0.082 ms); first new block Some(1184760400) for 1184760400 (+0 ticks); 0 old blocks drained; other []
B8 restart: rx_open took 0.745 ms
B8 restart cold (rate change, new streamer) lead 1 ms: stop→start issued 202.169 ms (apply 0.064 ms); first new block Some(1265933400) for 1265933400 (+0 ticks); 0 old blocks drained; other []
```

- **The device restarts on the requested tick at every lead down to 1 ms**, warm, after a rate change, and with a new streamer (`rx_open` 0.67–0.99 ms; the "stop→start issued 202 ms" of that row is the test's own drain of two 100 ms timeouts). The stop takes effect within ~0.4 ms (the old stream's 1–2 tail blocks).
- So spec 18 §3's restart lead of 50 ms is not the device's need; what made the Module's `cold` restart late is uhd-rx's loop (above).

### The device lead — `hw_b8_raw_device_lead`

A 100-sample burst at `time_now() + lead`, 8 times per lead (the lead counts from the time read's return, so it includes the send):

```
B8 device lead 3000 µs: 0/8 late; send took [3, 2, 2, 2, 3, 2, 2, 2] µs; outcomes ["BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 device lead 2000 µs: 0/8 late; send took [2, 2, 2, 2, 2, 2, 2, 2] µs; outcomes ["BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 device lead 1500 µs: 0/8 late; send took [2, 2, 2, 2, 2, 2, 2, 13] µs; outcomes ["BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 device lead 1000 µs: 0/8 late; send took [3, 11, 2, 1, 3, 2, 2, 7] µs; outcomes ["BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 device lead 700 µs: 0/8 late; send took [2, 1, 1, 7, 7, 7, 3, 1] µs; outcomes ["BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 device lead 500 µs: 0/8 late; send took [2, 2, 1, 2, 3, 2, 2, 8] µs; outcomes ["BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck"]
[UHD printed 34 × "L"] B8 device lead 300 µs: 1/8 late; send took [8, 9, 8, 8, 11, 8, 7, 7] µs; outcomes ["TimeError (×34)", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck", "BurstAck"]
[UHD printed 136 × "L"] B8 device lead 200 µs: 4/8 late; send took [2, 3, 7, 3, 10, 2, 7, 9] µs; outcomes ["BurstAck", "BurstAck", "TimeError (×33)", "BurstAck", "TimeError (×35)", "BurstAck", "TimeError (×34)", "TimeError (×34)"]
[UHD printed 276 × "L"] B8 device lead 100 µs: 8/8 late; send took [3, 8, 8, 8, 7, 7, 8, 8] µs; outcomes ["TimeError (×34)", "TimeError (×35)", "TimeError (×35)", "TimeError (×34)", "TimeError (×35)", "TimeError (×35)", "TimeError (×34)", "TimeError (×34)"]
[UHD printed 272 × "L"] B8 device lead 50 µs: 8/8 late; send took [12, 2, 3, 2, 2, 2, 6, 7] µs; outcomes ["TimeError (×35)", "TimeError (×34)", "TimeError (×35)", "TimeError (×34)", "TimeError (×33)", "TimeError (×34)", "TimeError (×33)", "TimeError (×34)"]
[UHD printed 267 × "L"] B8 device lead 0 µs: 8/8 late; send took [2, 13, 13, 12, 13, 11, 13, 13] µs; outcomes ["TimeError (×34)", "TimeError (×33)", "TimeError (×33)", "TimeError (×34)", "TimeError (×33)", "TimeError (×33)", "TimeError (×33)", "TimeError (×34)"]
```

- **0 of 8 late at 500 µs and more; 1/8 at 300 µs, 4/8 at 200 µs, 8/8 at 100 µs and less.** The device lead is about 0.3–0.5 ms on this bench; spec 18 §3's 2 ms has a margin of about 4 (unchanged: an input for Phase 8).
- A late 100-sample burst brings 33–35 `TimeError` reports (one per packet UHD sends, INFERRED) and is not played.

### Timed receive retunes — `hw_b8_raw_timed_retune`

A 100 kHz loopback tone; a receive retune by +50 kHz at `lead` from `time_now()` (0: untimed); the tone's frequency over 25-sample windows:

```
B8 retune lead 10000 µs: asked at 46878886 (issued at 44878886, apply 93 µs); the tone leaves 100 kHz at Some("+31.6 µs"), settles at 50 kHz at Some("+56.6 µs") (from the asked instant); before 99910 Hz, after 50114 Hz
B8 retune lead 5000 µs: asked at 119900895 (issued at 118900895, apply 144 µs); the tone leaves 100 kHz at Some("+37.5 µs"), settles at 50 kHz at Some("+62.5 µs") (from the asked instant); before 99989 Hz, after 50003 Hz
B8 retune lead 2000 µs: asked at 193281945 (issued at 192881945, apply 108 µs); the tone leaves 100 kHz at Some("+34.3 µs"), settles at 50 kHz at Some("+59.3 µs") (from the asked instant); before 99941 Hz, after 49887 Hz
B8 retune lead 1000 µs: asked at 267114668 (issued at 266914668, apply 105 µs); the tone leaves 100 kHz at Some("+44.7 µs"), settles at 50 kHz at Some("+444.7 µs") (from the asked instant); before 99836 Hz, after 49842 Hz
B8 retune lead 500 µs: asked at 340937192 (issued at 340837192, apply 115 µs); the tone leaves 100 kHz at Some("+37.0 µs"), settles at 50 kHz at Some("+437.0 µs") (from the asked instant); before 99875 Hz, after 49957 Hz
B8 retune lead 200 µs: asked at 414908323 (issued at 414868323, apply 115 µs); the tone leaves 100 kHz at Some("+52.4 µs"), settles at 50 kHz at Some("+452.4 µs") (from the asked instant); before 99885 Hz, after 50109 Hz
B8 retune lead 0 µs: asked at 488795434 (issued at 488795434, apply 74 µs); the tone leaves 100 kHz at Some("+184.8 µs"), settles at 50 kHz at Some("+434.8 µs") (from the asked instant); before 99940 Hz, after 50130 Hz
```

- **At leads of 2 ms and more the retune begins 31–52 µs after its instant and has settled by 57–62 µs.** At 1 ms and less it begins as early (37–52 µs) but the tone is disturbed until 437–452 µs: part of the tune landed late (INFERRED: an OBX tune is several register writes, the later ones past their time). Untimed, it begins 185 µs after the call. The Module's device lead of 2 ms is thus about the least lead for a clean timed retune on this bench.

### Queue depth of timed tunes — `hw_b8_raw_queue_depth`

Timed tunes all due 3 s ahead, one call each, alternating 999.6 and 1 000.4 MHz (nothing streamed, nothing emitted):

```
B8 queue Rx: call 8 took 2999.5 ms: Ok(Applied { rate: 200000000.0, freq: 1000400000.0, gain: 0.0 })
B8 queue Rx: apply ms per call ["0.21", "0.05", "0.04", "0.06", "0.03", "0.05", "0.03", "2999.50"]
B8 queue Rx: after the queue, untimed Ok(Applied { rate: 200000000.0, freq: 1000000000.0, gain: 0.0 })
B8 queue Tx: call 8 took 2999.6 ms: Ok(Applied { rate: 200000000.0, freq: 1000400000.0, gain: 0.0 })
B8 queue Tx: apply ms per call ["0.12", "0.06", "0.05", "0.06", "0.03", "0.05", "0.03", "2999.64"]
B8 queue Tx: after the queue, untimed Ok(Applied { rate: 200000000.0, freq: 1000000000.0, gain: 0.0 })
```

- **Seven timed OBX tunes queue; the eighth `apply` blocks until the queue drains (3 s)**, receive and transmit alike. The profiles' `command_queue_depth` is 16 (spec 18 UR-24, INFERRED): the device holds 7 tunes. The Module releases a held command only 3 ms ahead (UR-24's release window), so it queues few at a time; eight timed changes due within 3 ms would block uhd-control (INFERRED). An input for Phase 8 (the Mock's envelope claims 16).

### The loop's phase after a retune — `hw_b8_raw_timed_tune_phase`

Both LOs moved away, then back to `f0` (timed or untimed), then a 100 kHz tone burst at a fixed tick; the loop's phase against the tone, 6 cycles each:

```
B8 phase: sent 20000; received 26000 samples from Some((48760400, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(40927104, "Timeout")]
B8 phase 1000000000 Hz timed tune, cycle 0: 10000 samples, gain 0.0206, phase -137.3°
B8 phase: sent 20000; received 26000 samples from Some((124828200, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(117009639, "Timeout")]
B8 phase 1000000000 Hz timed tune, cycle 1: 10000 samples, gain 0.0206, phase -137.3°
B8 phase: sent 20000; received 26000 samples from Some((200891400, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(193068826, "Timeout")]
B8 phase 1000000000 Hz timed tune, cycle 2: 10000 samples, gain 0.0206, phase -137.3°
B8 phase: sent 20000; received 26000 samples from Some((276870400, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(269092928, "Timeout")]
B8 phase 1000000000 Hz timed tune, cycle 3: 10000 samples, gain 0.0206, phase -137.3°
B8 phase: sent 20000; received 26000 samples from Some((352879400, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(345128918, "Timeout")]
B8 phase 1000000000 Hz timed tune, cycle 4: 10000 samples, gain 0.0206, phase -137.3°
B8 phase: sent 20000; received 26000 samples from Some((429016400, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(421234829, "Timeout")]
B8 phase 1000000000 Hz timed tune, cycle 5: 10000 samples, gain 0.0206, phase -137.3°
B8 phase 1000000000 Hz timed tune: phases [-137.3, -137.3, -137.3, -137.3, -137.3, -137.3], largest difference from the first 0.0°
B8 phase: sent 20000; received 26000 samples from Some((505098800, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(497348587, "Timeout")]
B8 phase 1000000000 Hz untimed tune, cycle 0: 10000 samples, gain 0.0206, phase -137.8°
B8 phase: sent 20000; received 26000 samples from Some((581206800, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(573421982, "Timeout")]
B8 phase 1000000000 Hz untimed tune, cycle 1: 10000 samples, gain 0.0206, phase -137.8°
B8 phase: sent 20000; received 26000 samples from Some((657241800, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(649397981, "Timeout")]
B8 phase 1000000000 Hz untimed tune, cycle 2: 10000 samples, gain 0.0206, phase -137.7°
B8 phase: sent 20000; received 26000 samples from Some((733273000, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(725422682, "Timeout")]
B8 phase 1000000000 Hz untimed tune, cycle 3: 10000 samples, gain 0.0206, phase -137.8°
B8 phase: sent 20000; received 26000 samples from Some((809224800, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(801367455, "Timeout")]
B8 phase 1000000000 Hz untimed tune, cycle 4: 10000 samples, gain 0.0206, phase -137.8°
B8 phase: sent 20000; received 26000 samples from Some((885253800, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(877379843, "Timeout")]
B8 phase 1000000000 Hz untimed tune, cycle 5: 10000 samples, gain 0.0206, phase -137.7°
B8 phase 1000000000 Hz untimed tune: phases [-137.8, -137.8, -137.7, -137.8, -137.8, -137.7], largest difference from the first 0.1°
B8 phase: sent 20000; received 26000 samples from Some((961297800, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(953457360, "Timeout")]
B8 phase 1000123400 Hz timed tune, cycle 0: 10000 samples, gain 0.0206, phase 30.6°
B8 phase: sent 20000; received 26000 samples from Some((1037389200, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(1029639614, "Timeout")]
B8 phase 1000123400 Hz timed tune, cycle 1: 10000 samples, gain 0.0206, phase 30.7°
B8 phase: sent 20000; received 26000 samples from Some((1113487800, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(1105747569, "Timeout")]
B8 phase 1000123400 Hz timed tune, cycle 2: 10000 samples, gain 0.0206, phase 30.6°
B8 phase: sent 20000; received 26000 samples from Some((1189576400, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(1181755548, "Timeout")]
B8 phase 1000123400 Hz timed tune, cycle 3: 10000 samples, gain 0.0206, phase 30.7°
B8 phase: sent 20000; received 26000 samples from Some((1265565600, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(1257723347, "Timeout")]
B8 phase 1000123400 Hz timed tune, cycle 4: 10000 samples, gain 0.0206, phase 30.7°
B8 phase: sent 20000; received 26000 samples from Some((1341539600, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(1333711828, "Timeout")]
B8 phase 1000123400 Hz timed tune, cycle 5: 10000 samples, gain 0.0206, phase 30.7°
B8 phase 1000123400 Hz timed tune: phases [30.6, 30.7, 30.6, 30.7, 30.7, 30.7], largest difference from the first 0.0°
B8 phase: sent 20000; received 26000 samples from Some((1417537600, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(1409685759, "Timeout")]
B8 phase 1000123400 Hz untimed tune, cycle 0: 10000 samples, gain 0.0206, phase -83.6°
B8 phase: sent 20000; received 26000 samples from Some((1493582200, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(1485793351, "Timeout")]
B8 phase 1000123400 Hz untimed tune, cycle 1: 10000 samples, gain 0.0206, phase 58.4°
B8 phase: sent 20000; received 26000 samples from Some((1569624200, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(1561761909, "Timeout")]
B8 phase 1000123400 Hz untimed tune, cycle 2: 10000 samples, gain 0.0206, phase 20.6°
B8 phase: sent 20000; received 26000 samples from Some((1645691600, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(1637847703, "Timeout")]
B8 phase 1000123400 Hz untimed tune, cycle 3: 10000 samples, gain 0.0206, phase 47.8°
B8 phase: sent 20000; received 26000 samples from Some((1721740200, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(1713915874, "Timeout")]
B8 phase 1000123400 Hz untimed tune, cycle 4: 10000 samples, gain 0.0206, phase -15.4°
B8 phase: sent 20000; received 26000 samples from Some((1797809200, 2000)), rms 0.00722; tx reports ["BurstAck"]; other [(1789959364, "Timeout")]
B8 phase 1000123400 Hz untimed tune, cycle 5: 10000 samples, gain 0.0206, phase 164.8°
B8 phase 1000123400 Hz untimed tune: phases [-83.6, 58.4, 20.6, 47.8, -15.4, 164.8], largest difference from the first 142.0°
```

- **At a frequency the synthesizers reach in fractional-N (1 000.1234 MHz), a timed tune returns the loop's phase to 0.0–0.1°; an untimed one leaves it anywhere (spread 142°).** `phase_behavior_on_retune: random_unless_timed_tune` is true of the OBX on this bench (UHD's `_sync_phase`, design-notes §10).
- At 1 GHz the phase repeats to 0.1° even untimed (INFERRED: an integer-N frequency, where the LO's phase is fixed by the reference).
- The loop's amplitude gain from the tone: 0.0206 (−33.7 dB) at 1 GHz here, against 0.015 for the PN of B6 (the PN's bandwidth sees the IF filter; INFERRED).

### The in-flight window — `hw_b8_raw_in_flight_window`

A 2 s continuous burst at 2 Msps, each 0.5 ms buffer sent `window` before it plays:

```
B8 in-flight window 10000 µs: underflow 0, underflow in packet 0, time error 0, seq error 0, burst ack 1; first reports [TxReport { code: BurstAck, tick: Some(432470955), channel: 0 }]
B8 in-flight window 5000 µs: underflow 0, underflow in packet 0, time error 0, seq error 0, burst ack 1; first reports [TxReport { code: BurstAck, tick: Some(883035804), channel: 0 }]
B8 in-flight window 3000 µs: underflow 0, underflow in packet 0, time error 0, seq error 0, burst ack 1; first reports [TxReport { code: BurstAck, tick: Some(1333618929), channel: 0 }]
B8 in-flight window 2000 µs: underflow 0, underflow in packet 0, time error 0, seq error 0, burst ack 1; first reports [TxReport { code: BurstAck, tick: Some(1784209573), channel: 0 }]
B8 in-flight window 1000 µs: underflow 0, underflow in packet 0, time error 0, seq error 0, burst ack 1; first reports [TxReport { code: BurstAck, tick: Some(2234737048), channel: 0 }]
B8 in-flight window 500 µs: underflow 0, underflow in packet 0, time error 0, seq error 0, burst ack 1; first reports [TxReport { code: BurstAck, tick: Some(2685208201), channel: 0 }]
UB8 in-flight window 250 µs: underflow 1, underflow in packet 0, time error 0, seq error 0, burst ack 1; first reports [TxReport { code: Underflow, tick: Some(2949898664), channel: 0 }, TxReport { code: BurstAck, tick: Some(3135702341), channel: 0 }]
```

- **No underflow at 500 µs ahead and more; one at 250 µs.** Spec 18 §3's 10 ms window has a margin of about 20 at 2 Msps on this host.

### Back-to-back bursts — `hw_b8_raw_burst_gap`: **a timed start at the previous burst's end is late**

A 1 000-sample burst ending with end-of-burst at tick T, then a 100-sample burst with a timed start-of-burst at T + gap samples, 4 times each:

```
[UHD printed 132 × "L"] B8 burst gap 0 samples: ["BurstAck+TimeError", "BurstAck+TimeError", "BurstAck+TimeError", "BurstAck+TimeError"]
B8 burst gap 1 samples: ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 burst gap 2 samples: ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 burst gap 3 samples: ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 burst gap 5 samples: ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 burst gap 10 samples: ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 burst gap 20 samples: ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 burst gap 50 samples: ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 burst gap 100 samples: ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 burst gap 1000 samples: ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
```

- **At gap 0 the second burst is reported late (`TimeError`) and not played, 4 of 4; at a gap of 1 sample and more, never.** Spec 18 UR-23 makes exactly the gap-0 transition when a held burst starts where the open one is cut ("the current one is sent up to the sample before the next start, its last buffer with end-of-burst … the next starts with start-of-burst and its time spec"). See the next test; design-notes §11 F3.

### Preemption of a running repeat — `hw_b8_preemption`

A repeat running; a 100-sample burst `send` at `lead` ahead:

```
[UHD printed 33 × "L"] B8 preempt lead 20 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(15), "outcome": String("late_at_device"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(121809)}}]
B8 preempt lead 20 ms: asked 121809; bursts [{"actual_start":null,"blocks":117,"end":"eob","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402631574},"samples":116642,"target":{"domain":{"local":4,"node":0},"ticks":5167},"wraps":116},{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":100,"target":{"domain":{"local":4,"node":0},"ticks":121809},"wraps":1}]
B8 preempt lead 10 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(408000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(111623)}}]
B8 preempt lead 10 ms: asked 111623; bursts [{"actual_start":null,"blocks":209,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402173544},"samples":209000,"target":{"domain":{"local":4,"node":0},"ticks":5031},"wraps":209}]
B8 preempt lead 5 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(6277000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(107088)}}]
B8 preempt lead 5 ms: asked 107088; bursts [{"actual_start":null,"blocks":210,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402430829},"samples":210000,"target":{"domain":{"local":4,"node":0},"ticks":5365},"wraps":210}]
B8 preempt lead 3 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(8224000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(104843)}}]
B8 preempt lead 3 ms: asked 104843; bursts [{"actual_start":null,"blocks":210,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402461188},"samples":210000,"target":{"domain":{"local":4,"node":0},"ticks":5067},"wraps":210}]
```

- **At 20 ms the Module preempts as UR-21/UR-23 say — the repeat ends with `eob` at 121 809, exactly the burst's start — and the device reports the burst late (`late_at_device`, `late_by_ns` 15) and does not play it** (UHD's 33 `L`s): the gap-0 case above. The `TIME_ERROR` reports it; the burst's record still reads `end: eob, late_by: null`.
- At 10, 5 and 3 ms UR-21 drops the burst (`cause: late, outcome: drop`, late by 0.41, 6.28, 8.22 ms): so a preempting burst needs a lead of about 10.4–11.3 ms from submission (lead + late_by at the three leads): the in-flight window (10 ms) plus the delivery and uhd-tx's position in its buffers.

### The transmit end after `Stop` — `hw_b8_stop_end`

A repeat, a capture across it, `Stop` for `radio/tx` 50 ms into the capture; where the loop stops hearing it (100-sample windows, less the 44-sample loop delay):

```
B8 stop end, trial 0: Stop submitted at root 415679130; the loop heard the repeat until root 417821400 (+10.711 ms from the Stop); steady power 1.24e-4, after 7.95e-9; bursts [{"actual_start":null,"blocks":77,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402425430},"samples":77000,"target":{"domain":{"local":4,"node":0},"ticks":5151},"wraps":77}]
B8 stop end, trial 0: tx clocks [(401395400, 200)]
B8 stop end, trial 1: Stop submitted at root 415476294; the loop heard the repeat until root 417746400 (+11.351 ms from the Stop); steady power 1.24e-4, after 8.12e-9; bursts [{"actual_start":null,"blocks":78,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402142599},"samples":78000,"target":{"domain":{"local":4,"node":0},"ticks":5031},"wraps":78}]
B8 stop end, trial 1: tx clocks [(401136400, 200)]
B8 stop end, trial 2: Stop submitted at root 415609792; the loop heard the repeat until root 417667400 (+10.288 ms from the Stop); steady power 1.24e-4, after 7.56e-9; bursts [{"actual_start":null,"blocks":77,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402275704},"samples":77000,"target":{"domain":{"local":4,"node":0},"ticks":5031},"wraps":77}]
B8 stop end, trial 2: tx clocks [(401269600, 200)]
```

- **The transmission ends 10.3–11.4 ms after the Stop's submission**, and where the burst record says it does (the record's end `target + samples` on the transmit clock, e.g. trial 0: 401 395 400 + (5 151 + 77 000) · 200 = 417 825 600, heard until 417 821 400, one 100-sample window). UR-23 bounds it by `s + 10 ms` from the instant uhd-tx takes the Stop; from the submission it adds the delivery (≤ 1.4 ms here). After it the capture's power is 7.6–8.1e-9 against 1.24e-4 (−42 dB: nothing).

### The receive rate — `hw_perf_rx_rates`

A receive Run of 3 s at each rate, no capture written (the Sink consumes the blocks):

```
B8 rate 10 Msps: Ok(()); termination Stopped { cause: Client }; overflows 0; stats {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":15008,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":30015627,"tx_bursts":0,"tx_samples":0}
B8 rate 20 Msps: Ok(()); termination Stopped { cause: Client }; overflows 0; stats {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":30023,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":60045170,"tx_bursts":0,"tx_samples":0}
B8 rate 25 Msps: Ok(()); termination Stopped { cause: Client }; overflows 0; stats {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":37521,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":75040674,"tx_bursts":0,"tx_samples":0}
B8 rate 40 Msps: Ok(()); termination Stopped { cause: Client }; overflows 0; stats {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":60042,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":120082466,"tx_bursts":0,"tx_samples":0}
B8 rate 50 Msps: Ok(()); termination Stopped { cause: Client }; overflows 0; stats {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":75042,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":150082602,"tx_bursts":0,"tx_samples":0}
B8 rate 100 Msps: Ok(()); termination Stopped { cause: Client }; overflows 0; stats {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":150102,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":300203246,"tx_bursts":0,"tx_samples":0}
B8 rate 200 Msps: Ok(()); termination Stopped { cause: Client }; overflows 0; stats {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":300195,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":600389384,"tx_bursts":0,"tx_samples":0}
```

- **10 to 200 Msps for 3 s: no overflow, no link drop, no off-lattice block** (600 389 384 samples at 200 Msps). The profiles' `rx_bytes_per_s` is 1e9 (250 Msps at 4 bytes a sample); 200 Msps, 800 MB/s, is what the X300's master clock allows. With a capture written to disk the numbers would be the disk's (not measured).

### The loop delay by rate — `hw_b6_delay_by_rate`

B6's burst at each rate the bench may emit at:

```
B6 400 ksps: 5000 samples, rms 0.00474; correlation peak Some((35, 0.017011456, 20.078028)) (offset, gain, phase °); peak over median 4073.2 (needs > 8)
B6 400 ksps: delay Some(35) samples = Some(87.5) µs; TIME_ERROR 0
B6 500 ksps: 5000 samples, rms 0.00495; correlation peak Some((44, 0.019085078, 22.403013)) (offset, gain, phase °); peak over median 4715.2 (needs > 8)
B6 500 ksps: delay Some(44) samples = Some(88.0) µs; TIME_ERROR 0
B6 1000 ksps: 5000 samples, rms 0.00493; correlation peak Some((44, 0.015061438, 25.127499)) (offset, gain, phase °); peak over median 3127.4 (needs > 8)
B6 1000 ksps: delay Some(44) samples = Some(44.0) µs; TIME_ERROR 0
B6 2000 ksps: 5000 samples, rms 0.00473; correlation peak Some((36, 0.017465504, 20.878965)) (offset, gain, phase °); peak over median 3112.9 (needs > 8)
B6 2000 ksps: delay Some(36) samples = Some(18.0) µs; TIME_ERROR 0
```

- **35 samples at 400 ksps, 44 at 500 ksps, 44 at 1 Msps, 36 at 2 Msps** (87.5, 88, 44, 18 µs): neither a fixed sample count nor a fixed time (the rates' filter chains differ; INFERRED). The profiles' `tx_path_delay_samples` is one number, 45: Phase 8 needs a value per rate (or per decimation).

### B9 without the unplug — `hw_b9_long_receive`, `hw_b9_transmit_only_session`

```
B9 long receive: Ok(()); termination Stopped { cause: Client }; DEVICE_LOST []; overflows 0; stats {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":600009,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":1200017333,"tx_bursts":0,"tx_samples":0}
B9 transmit only: radio.rx.channels 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }
B9 transmit only: termination Stopped { cause: Client }; DEVICE_LOST []; TX_UNDERFLOW 0; timing [{"at":387577,"start_up_until":400387577,"what":"arm"},{"lead_ns":1999997335,"t0":400517200,"what":"start"},{"channels":1,"dir":"tx","origin":401306600,"what":"enabled"},{"host_delay_ms":2003,"index":0,"what":"first_rx_block"},{"booked_at":401571902,"e1":411572000,"e2":null,"key":"radio.rx.channels","what":"cold_change"},{"e1":411572000,"e2":null,"what":"rx_switch"},{"at":12401755973,"done":12422857496,"mode":"Orderly","what":"stop"}]
```

- **A 120 s receive Run at 10 Msps (1.2 × 10⁹ samples): no `DEVICE_LOST`, no overflow, no link drop, ended by the client.** No `UHD_ERROR_RUNTIME` false positive over two minutes (UR-29's worry).
- **A transmit-only Session for 60 s** (the receiver switched off with `radio.rx.channels = 0`, admitted as a `cold` change with no `e2`; nothing sent): no `DEVICE_LOST`, no `TX_UNDERFLOW`, ended by the client. The unplug itself during either stays for the owner at the bench.

### Authority drift over 60 s — `hw_b2_drift_60s`

```
B2 drift after 10.000812179s: host-derived 2000070169 ticks, device 2000087280 ticks, difference -17111 ticks (-8.56 ppm)
B2 drift after 20.00125855s: host-derived 4000177088 ticks, device 4000200029 ticks, difference -22941 ticks (-5.73 ppm)
B2 drift after 30.001839729s: host-derived 6000272216 ticks, device 6000293694 ticks, difference -21478 ticks (-3.58 ppm)
B2 drift after 40.002382363s: host-derived 8000394655 ticks, device 8000410128 ticks, difference -15473 ticks (-1.93 ppm)
B2 drift after 50.002919483s: host-derived 10000501785 ticks, device 10000522687 ticks, difference -20902 ticks (-2.09 ppm)
B2 drift after 60.003482287s: host-derived 12000597403 ticks, device 12000620033 ticks, difference -22630 ticks (-1.89 ppm)
```

- **The Authority's host-derived time runs 77–115 µs behind the device's, and the difference does not grow over 60 s** (−17 111 … −22 630 ticks): it re-anchors, so there is no ppm drift to bound, only an offset of about 0.1 ms, within B2's host-bracket uncertainty (306 µs, session 1). The "ppm" column is therefore not a drift.

### B1–B8 once more at `85ac77b`

With the envelope in every transmitting step's profile and all the tests above in the tree. All eight pass; no `DEVICE_LOST` in any output:

```
B1 describe: {
B1 profile: x310-obx
B1 Rx: 2 channels, front ends [Ok("OBX RX"), Ok("Unknown (0xffff) - 0")]
B1 Tx: 2 channels, front ends [Ok("OBX TX"), Ok("Unknown (0xffff) - 0")]
B1 ref_locked: Ok(Some(false))
B1 time advanced 20089344 ticks in 100 ms
B1 time advanced 20127787 ticks in 100 ms
B1 time advanced 20114969 ticks in 100 ms
B1 tx 0: Ok(Applied { rate: 200000000.0, freq: 10000000.000000238, gain: 0.0 })
B1 tx 1: Ok(Applied { rate: 200000000.0, freq: 0.0, gain: 0.0 })
B2 lateness µs: median 513 max 614
B2 relations: [
B2 anchor drift over 10.000878905s: -26271 ticks
B3 applied: [{"at":{"domain":{"local":2,"node":0},"ticks":346440},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.rx.sample_rate_hz","read_back":1000000.0},{"at":{"domain":{"local":2,"node":0},"ticks":346440},"channel":0,"claimed":1000000000.0,"difference":0.0,"key":"radio.rx.frequency_hz","read_back":1000000000.0},{"at":{"domain":{"local":2,"node":0},"ticks":346440},"channel":0,"claimed":0.0,"difference":0.0,"key":"radio.rx.gain_db","read_back":0.0},{"at":{"domain":{"local":2,"node":0},"ticks":346440},"channel":0,"claimed":"RX2","key":"radio.rx.antenna"}]
B3 timing: [{"at":420659,"start_up_until":400420659,"what":"arm"},{"lead_ns":1999996815,"t0":400595000,"what":"start"},{"host_delay_ms":2003,"index":0,"what":"first_rx_block"},{"at":403228328,"until":403428328,"what":"rx_stop"},{"at":404164005,"cut":403428328,"what":"rx_stop_untimed"},{"at":403228328,"done":423696476,"mode":"Orderly","what":"stop"}]

B5 no overflow with a 300 ms stall; the socket buffer absorbed it
OB5 overflowed with a 1000 ms stall: {"link_drops_seen":1373,"rx_before_origin":0,"rx_blocks":7733,"rx_errors":0,"rx_off_lattice":2178,"rx_overflows":1,"rx_overlapping":0,"rx_samples":15462924,"tx_bursts":0,"tx_samples":0}
B5 1256 gaps: 1255 link drops of 2746000 samples in all, from sample Some(5234000) to Some(10982000)
B5 gap OverflowRestart at receive sample 11108408 for 4557377 samples (455.7377 ms at 10 Msps), lost Some(4557377)
B5 capture 0 … 20020301, 1257 valid segment(s)
B5 timing [{"at":277997,"start_up_until":400277997,"what":"arm"},{"lead_ns":1999997430,"t0":400393700,"what":"start"},{"host_delay_ms":2000,"index":0,"what":"first_rx_block"},{"ms":1000,"what":"rx_stall"},{"at":800599710,"until":800799710,"what":"rx_stop"},{"at":800823699,"cut":800799710,"what":"rx_stop_untimed"},{"at":800599710,"done":821273075,"mode":"Orderly","what":"stop"}]
B5 sample clocks: [SampleClockRecord { stream: ResourceId { node: NodeId(0), path: "usrp/rx" }, domain: ClockDomainId { node: NodeId(0), local: 3 }, root: ClockDomainId { node: NodeId(0), local: 2 }, root_ticks_per_tick: Rational { num: 20, den: 1 }, origin: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 400393700 }, ended_at: None, nominal_rate: Rational { num: 10000000, den: 1 } }]
B5 RX_OVERFLOW: Ok(RxOverflowPayload { cause: Overrun, lost: 4557377, restart_gap_ns: 455737700 })
B6 burst: 5000 samples, rms 0.00490; correlation peak Some((44, 0.014989309, 25.364481)) (offset, gain, phase °); peak over median 2973.7 (needs > 8)
B6: transmit-to-receive delay 44 samples
B6 repeat: 4000 samples, rms 0.01100; correlation peak Some((44, 0.015289056, 24.842901)) (offset, gain, phase °); peak over median 28.8 (needs > 8)
B6 repeat: 3 whole periods from sample 44 (offset, gain, phase °): [(44, 0.015289056, 24.842901), (1044, 0.015283262, 24.680117), (2044, 0.015288842, 24.655783)]
B6 bursts: [{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":1000,"target":{"domain":{"local":4,"node":0},"ticks":2011996},"wraps":1}] / [{"actual_start":null,"blocks":20,"end":"stop","late_by":null,"requested_target":null,"samples":20000,"target":{"domain":{"local":4,"node":0},"ticks":2002742},"wraps":20}]
B7 refused: Rejected { violations: [Violation { check: Namespace("radio.rf_envelope"), key: Some(Key("radio.tx.frequency_hz")), requested: Some(Num(1100000000.0)), reason: "RM-19: radio: 1100000000 Hz is in no allowed band" }] }
B7 capture: 5000 samples, rms 0.01106; correlation peak Some((3800, 0.015282318, 24.471838)) (offset, gain, phase °); peak over median 28.9 (needs > 8)
B7 capture asked at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 411455955 }; artifact [ContinuityMap { domain: ClockDomainId { node: NodeId(0), local: 3 }, channels: 1, valid: [[Segment { start: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 54627 }, len: 5000 }]], gaps: [], channel_gaps: [], first: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 54627 }, end: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 59627 } }]
B7 sample clocks: [SampleClockRecord { stream: ResourceId { node: NodeId(0), path: "usrp/rx" }, domain: ClockDomainId { node: NodeId(0), local: 3 }, root: ClockDomainId { node: NodeId(0), local: 2 }, root_ticks_per_tick: Rational { num: 200, den: 1 }, origin: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 400530600 }, ended_at: None, nominal_rate: Rational { num: 1000000, den: 1 } }, SampleClockRecord { stream: ResourceId { node: NodeId(0), path: "usrp/tx" }, domain: ClockDomainId { node: NodeId(0), local: 4 }, root: ClockDomainId { node: NodeId(0), local: 2 }, root_ticks_per_tick: Rational { num: 200, den: 1 }, origin: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 401194600 }, ended_at: None, nominal_rate: Rational { num: 1000000, den: 1 } }]
B7 time errors: []
B8 lead 10000 µs: TIME_ERROR []
B8 lead 10000 µs: timing [{"at":378710,"start_up_until":400378710,"what":"arm"},{"lead_ns":1999996980,"t0":400621800,"what":"start"},{"channels":1,"dir":"tx","origin":401192000,"what":"enabled"},{"host_delay_ms":2004,"index":0,"what":"first_rx_block"},{"at":421526477,"until":421726477,"what":"rx_stop"},{"at":422212034,"cut":421726477,"what":"rx_stop_untimed"},{"at":421526477,"done":442791555,"mode":"Orderly","what":"stop"}]
B8 lead 10000 µs: bursts [{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":100,"target":{"domain":{"local":4,"node":0},"ticks":10021},"wraps":1}]
B8 lead 5000 µs: TIME_ERROR []
B8 lead 5000 µs: timing [{"at":411588,"start_up_until":400411588,"what":"arm"},{"lead_ns":1999996745,"t0":400562200,"what":"start"},{"channels":1,"dir":"tx","origin":401215200,"what":"enabled"},{"host_delay_ms":2003,"index":0,"what":"first_rx_block"},{"at":421619176,"until":421819176,"what":"rx_stop"},{"at":422524083,"cut":421819176,"what":"rx_stop_untimed"},{"at":421619176,"done":442556310,"mode":"Orderly","what":"stop"}]
B8 lead 5000 µs: bursts [{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":100,"target":{"domain":{"local":4,"node":0},"ticks":5137},"wraps":1}]
B8 lead 3000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(126000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(3062)}}]
B8 lead 3000 µs: timing [{"at":297153,"start_up_until":400297153,"what":"arm"},{"lead_ns":1999997800,"t0":400456000,"what":"start"},{"channels":1,"dir":"tx","origin":401156000,"what":"enabled"},{"host_delay_ms":2004,"index":0,"what":"first_rx_block"},{"at":421485967,"until":421685967,"what":"rx_stop"},{"at":422445561,"cut":421685967,"what":"rx_stop_untimed"},{"at":421485967,"done":442519834,"mode":"Orderly","what":"stop"}]
B8 lead 3000 µs: bursts []
B8 lead 2000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(1205000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(2030)}}]
B8 lead 2000 µs: timing [{"at":337884,"start_up_until":400337884,"what":"arm"},{"lead_ns":1999997705,"t0":400534000,"what":"start"},{"channels":1,"dir":"tx","origin":401240200,"what":"enabled"},{"host_delay_ms":2003,"index":0,"what":"first_rx_block"},{"at":421647584,"until":421847584,"what":"rx_stop"},{"at":422492753,"cut":421847584,"what":"rx_stop_untimed"},{"at":421647584,"done":442915902,"mode":"Orderly","what":"stop"}]
B8 lead 2000 µs: bursts []
B8 lead 1500 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(1427000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(1641)}}]
B8 lead 1500 µs: timing [{"at":268565,"start_up_until":400268565,"what":"arm"},{"lead_ns":1999997740,"t0":400428400,"what":"start"},{"channels":1,"dir":"tx","origin":401203400,"what":"enabled"},{"host_delay_ms":2003,"index":0,"what":"first_rx_block"},{"at":421625410,"until":421825410,"what":"rx_stop"},{"at":422383610,"cut":421825410,"what":"rx_stop_untimed"},{"at":421625410,"done":442500443,"mode":"Orderly","what":"stop"}]
B8 lead 1500 µs: bursts []
B8 lead 1000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(2220000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(1029)}}]
B8 lead 1000 µs: timing [{"at":273722,"start_up_until":400273722,"what":"arm"},{"lead_ns":1999997725,"t0":400500200,"what":"start"},{"channels":1,"dir":"tx","origin":401280200,"what":"enabled"},{"host_delay_ms":2004,"index":0,"what":"first_rx_block"},{"at":421640066,"until":421840066,"what":"rx_stop"},{"at":422473685,"cut":421840066,"what":"rx_stop_untimed"},{"at":421640066,"done":442670671,"mode":"Orderly","what":"stop"}]
B8 lead 1000 µs: bursts []
B8 lead 500 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(2525000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(542)}}]
B8 lead 500 µs: timing [{"at":315343,"start_up_until":400315343,"what":"arm"},{"lead_ns":1999995835,"t0":400488000,"what":"start"},{"channels":1,"dir":"tx","origin":401280800,"what":"enabled"},{"host_delay_ms":2004,"index":0,"what":"first_rx_block"},{"at":421630157,"until":421830157,"what":"rx_stop"},{"at":422461254,"cut":421830157,"what":"rx_stop_untimed"},{"at":421630157,"done":442688871,"mode":"Orderly","what":"stop"}]
B8 lead 500 µs: bursts []
```

(B1's `describe` and B7's clocks are in the logs; the lines above are the tests' `B…` lines.)

### Part 2 in short

Three findings need the owner (design-notes §11): **F1** the X300 ignores a timed receive stop, so a `cold` receive change loses ~50 ms before `e1` without a flag; **F2** the restart after it is late, because uhd-rx waits for a 100 ms receive timeout; **F3** a burst whose timed start is the previous burst's end tick is dropped by the device, which is how UR-23 preempts. Phase 8 inputs (spec 18 §3 and the profiles unchanged): device lead 0.3–0.5 ms (Module 2 ms); a clean timed retune needs ~2 ms; restart lead ≥ 1 ms on the device (Module 50 ms); queue depth 7 OBX tunes (profile 16); in-flight window ≥ 0.5 ms at 2 Msps (Module 10 ms); transmit end ≤ 11.4 ms after the Stop's submission; receive 200 Msps sustained; loop delay 35–44 samples by rate (profile 45); timed tunes restore the loop phase (fractional-N), untimed do not; no false `DEVICE_LOST` in 120 s receive and 60 s transmit-only; the Authority's offset bounded at ~0.1 ms. Test code only (`85ac77b`); no Module code changed.

## Session 2, part 3 — after the fix of design-notes §11 F1–F3 (`bc0db98`)

The owner: "F1-F3は推奨で". The fix is design-notes §11 "The decision and the change"; spec 18 UR-23, UR-25 and the rest follow. Then on the bench, at `bc0db98` (release server rebuilt from it for B7). Logs in `~/ezsdr-bench/s2-fixed/`.

### The two findings, again — fixed on the bench

`hw_b8_cold_change_capture` (the test unchanged but for its comment):

```
B8 cold: capture asked at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 424541171 }; rate change submitted at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 432682534 }: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }
B8 cold: termination Stopped { cause: Client }
B8 cold: sample clocks [SampleClockRecord { stream: ResourceId { node: NodeId(0), path: "usrp/rx" }, domain: ClockDomainId { node: NodeId(0), local: 3 }, root: ClockDomainId { node: NodeId(0), local: 2 }, root_ticks_per_tick: Rational { num: 200, den: 1 }, origin: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 400465400 }, ended_at: Some(TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 442727400 }), nominal_rate: Rational { num: 1000000, den: 1 } }, SampleClockRecord { stream: ResourceId { node: NodeId(0), path: "usrp/rx" }, domain: ClockDomainId { node: NodeId(0), local: 4 }, root: ClockDomainId { node: NodeId(0), local: 2 }, root_ticks_per_tick: Rational { num: 100, den: 1 }, origin: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 452727400 }, ended_at: None, nominal_rate: Rational { num: 2000000, den: 1 } }]
B8 cold: timing [{"at":315082,"start_up_until":400315082,"what":"arm"},{"lead_ns":1999997620,"t0":400465400,"what":"start"},{"host_delay_ms":2003,"index":0,"what":"first_rx_block"},{"booked_at":432727252,"e1":442727400,"e2":452727400,"key":"radio.rx.sample_rate_hz","what":"cold_change"},{"at":443012742,"cut":442727400,"what":"rx_stop_untimed"},{"at":443073534,"e1":442727400,"e2":452727400,"what":"rx_switch"},{"at":524321910,"until":524521910,"what":"rx_stop"},{"at":524581715,"cut":524521910,"what":"rx_stop_untimed"},{"at":524321910,"done":544874121,"mode":"Orderly","what":"stop"}]
B8 cold: applied [{"at":{"domain":{"local":2,"node":0},"ticks":277793},"channel":0,"claimed":1000000.0,"difference":0.0,"key":"radio.rx.sample_rate_hz","read_back":1000000.0},{"at":{"domain":{"local":2,"node":0},"ticks":277793},"channel":0,"claimed":1000000000.0,"difference":0.0,"key":"radio.rx.frequency_hz","read_back":1000000000.0},{"at":{"domain":{"local":2,"node":0},"ticks":277793},"channel":0,"claimed":0.0,"difference":0.0,"key":"radio.rx.gain_db","read_back":0.0},{"at":{"domain":{"local":2,"node":0},"ticks":277793},"channel":0,"claimed":"RX2","key":"radio.rx.antenna"},{"at":{"domain":{"local":2,"node":0},"ticks":442727400},"issued":{"domain":{"local":2,"node":0},"ticks":443012742},"key":"rx_stop"},{"at":{"domain":{"local":2,"node":0},"ticks":443068134},"channel":0,"claimed":2000000.0,"difference":0.0,"key":"radio.rx.sample_rate_hz","read_back":2000000.0},{"at":{"domain":{"local":2,"node":0},"ticks":443068134},"channel":0,"claimed":1000000000.0,"difference":0.0,"key":"radio.rx.frequency_hz","read_back":1000000000.0},{"at":{"domain":{"local":2,"node":0},"ticks":443068134},"channel":0,"claimed":0.0,"difference":0.0,"key":"radio.rx.gain_db","read_back":0.0},{"at":{"domain":{"local":2,"node":0},"ticks":443068134},"channel":0,"claimed":"RX2","key":"radio.rx.antenna"},{"at":{"domain":{"local":2,"node":0},"ticks":524521910},"issued":{"domain":{"local":2,"node":0},"ticks":524581715},"key":"rx_stop"}]
B8 cold: stats {"link_drops_seen":0,"rx_before_origin":1,"rx_blocks":465,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":929256,"tx_bursts":0,"tx_samples":0}
B8 cold: artifact rec_0 continuity [ContinuityMap { domain: ClockDomainId { node: NodeId(0), local: 3 }, channels: 1, valid: [[Segment { start: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 120379 }, len: 90931 }]], gaps: [], channel_gaps: [], first: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 120379 }, end: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 3 }, ticks: 211310 } }, ContinuityMap { domain: ClockDomainId { node: NodeId(0), local: 4 }, channels: 1, valid: [[Segment { start: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 0 }, len: 109069 }]], gaps: [], channel_gaps: [], first: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 0 }, end: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 109069 } }]
B8 cold: event sink.CAPTURE_WRITTEN from ResourceId { node: NodeId(0), path: "sink/rec" } at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 464150609 }: {"artifact":{"continuity":[{"channel_gaps":[],"channels":1,"domain":{"local":3,"node":0},"end":{"domain":{"local":3,"node":0},"ticks":211310},"first":{"domain":{"local":3,"node":0},"ticks":120379},"gaps":[],"valid":[[{"len":90931,"start":{"domain":{"local":3,"node":0},"ticks":120379}}]]},{"channel_gaps":[],"channels":1,"domain":{"local":4,"node":0},"end":{"domain":{"local":4,"node":0},"ticks":109069},"first":{"domain":{"local":4,"node":0},"ticks":0},"gaps":[],"valid":[[{"len":109069,"start":{"domain":{"local":4,"node":0},"ticks":0}}]]}],"hash":"sha256:2c60f7525e21580707fa1cd4d14cb9af87a305cdaf78d561acf5fa0b8b4091d2","id":"rec_0","kind":"sink.capture","marks":[],"partial":false,"size_bytes":1600000,"uri":"file:///tmp/ezsdr-uhd-120-0/local_78_18da14b12bf85e5d-0_rec_0.sigmf-data"},"request":0}
```

- **The old clock's samples reach `e₁`:** the capture's first map ends at receive sample 211 310 = (442 727 400 − 400 465 400) / 200, the old clock's `ended_at`; before the fix it ended 49.8 ms early. **The new clock's start is on time:** its map begins at sample 0 (`e₂`), no gap, no `LATE_COMMAND` (no `radio.` event in the output).
- uhd-rx stopped the stream untimed 1.43 ms after `e₁` (`rx_stop_untimed` at 443 012 742, recorded in `applied` with `e₁`), and the switch ran at 443 073 534, 48.3 ms before `e₂`.

`hw_b8_preemption`:

```
B8 preempt lead 20 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []
B8 preempt lead 20 ms: asked 121673; bursts [{"actual_start":null,"blocks":117,"end":"eob","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402402964},"samples":116545,"target":{"domain":{"local":4,"node":0},"ticks":5128},"wraps":116},{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":100,"target":{"domain":{"local":4,"node":0},"ticks":121673},"wraps":1}]
B8 preempt lead 10 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(607000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(111559)}}]
B8 preempt lead 10 ms: asked 111559; bursts [{"actual_start":null,"blocks":209,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402125287},"samples":209000,"target":{"domain":{"local":4,"node":0},"ticks":5166},"wraps":209}]
B8 preempt lead 5 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(6450000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(106620)}}]
B8 preempt lead 5 ms: asked 106620; bursts [{"actual_start":null,"blocks":209,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402356963},"samples":209000,"target":{"domain":{"local":4,"node":0},"ticks":5070},"wraps":209}]
B8 preempt lead 3 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(7529000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(104537)}}]
B8 preempt lead 3 ms: asked 104537; bursts [{"actual_start":null,"blocks":210,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402204688},"samples":210000,"target":{"domain":{"local":4,"node":0},"ticks":5066},"wraps":210}]
```

- **At 20 ms the preempting burst is now played:** no `TIME_ERROR`, no `L` from UHD; the repeat ends at 121 673, the burst's start, and the burst's record follows it. Before the fix the device dropped it as late. At 10, 5 and 3 ms UR-21 drops the burst as before (late by 0.61, 6.45, 7.53 ms).

### B1–B8, B9's long Runs and B7 (Python) again: pass

| Step | Outcome at `bc0db98` |
|---|---|
| B1 `hw_b1_probe` | pass |
| B2 `hw_b2_authority` | pass (and 8 more runs, below) |
| B3 `hw_b3_receive_at_t0` | pass |
| B4 `hw_b4_capture_at_a_sample_index` | pass |
| B5 `hw_b5_overflow` | pass |
| B6 `hw_b6_txrx_and_repeat` | **first run failed before the Run: `UR-7: uhd_usrp_set_clock_source: UHD error 44: RuntimeError: Reference Clock PLL failed to lock to internal source.`** (below); the next two runs pass, delay 44 samples, the repeat back to back |
| B7 `hw_b7_session_loopback` | pass |
| B8 `hw_b8_leads`, `hw_b8_stop_end` | ran (as in part 2) |
| B9 `hw_b9_long_receive` | **first run failed the same way (reference PLL)**; the rerun passes: 120 s, 1.2 × 10⁹ samples, no `DEVICE_LOST`, no overflow |
| B9 `hw_b9_transmit_only_session` | pass |
| B7 (Python) `minimal.py`, `bench_loopback.py` | both run; no `TIME_ERROR`; the 2.4 GHz transmit retune refused by `radio.rf_envelope`; Manifests `session-7-4`, `session-7-5` |

B6's second run:

```
B6 burst: 5000 samples, rms 0.00493; correlation peak Some((44, 0.015061145, 26.156517)) (offset, gain, phase °); peak over median 3149.5 (needs > 8)
B6: transmit-to-receive delay 44 samples
B6 repeat: 4000 samples, rms 0.01106; correlation peak Some((2044, 0.015362606, 25.512928)) (offset, gain, phase °); peak over median 28.8 (needs > 8)
B6 repeat: 3 whole periods from sample 44 (offset, gain, phase °): [(44, 0.015360814, 25.71258), (1044, 0.015360472, 25.590353), (2044, 0.015362606, 25.512928)]
B6 bursts: [{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":1000,"target":{"domain":{"local":4,"node":0},"ticks":2011927},"wraps":1}] / [{"actual_start":null,"blocks":19,"end":"stop","late_by":null,"requested_target":null,"samples":19000,"target":{"domain":{"local":4,"node":0},"ticks":2003301},"wraps":19}]
```

`bench_loopback.py`:

```
sample rate 20000000.0 S/s (coerced, a new SampleClock)
the transmit retune outside the RF envelope was refused: radio.rf_envelope: RM-19: radio: 2400000000 Hz is in no allowed band
y: correlation peak at sample 15885; z: at sample 13174
rec_0: first sample {'domain': {'node': 0, 'local': 3}, 'ticks': 54679}
rec_1: first sample {'domain': {'node': 0, 'local': 3}, 'ticks': 179390}
asked: y at {'domain': {'node': 0, 'local': 2}, 'ticks': 411469185}, z at {'domain': {'node': 0, 'local': 2}, 'ticks': 436411268}
TIME_ERROR events: 0 (spike K6: none)
Manifest: /bench/ezsdr-runs/session-7-5/manifest.json
```

### A new finding: the reference PLL sometimes fails to lock (UR-7)

The failed B6 run, whole output:

```
running 1 test
thread 'hw_b6_txrx_and_repeat' (21) panicked at crates/ezsdr-radio-uhd/tests/common/mod.rs:232:144:
called `Result::unwrap()` on an `Err` value: "UR-7: uhd_usrp_set_clock_source: UHD error 44: RuntimeError: Reference Clock PLL failed to lock to internal source."
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
hw_b6_txrx_and_repeat --- FAILED
failures:
failures:
    hw_b6_txrx_and_repeat
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 25 filtered out; finished in 32.60s
error: test failed, to rerun pass `-p ezsdr-radio-uhd --test hardware`
```

- `DeviceAuthority::new` (UR-7) selects the clock source; UHD's `set_clock_source("internal")` waited ~30 s and returned `UHD error 44: RuntimeError: Reference Clock PLL failed to lock to internal source.` The same happened once to `hw_b9_long_receive`. **2 of about 24 device opens after the fix; none in the ~50 of session 2 before it, none in 8 further runs of `hw_b2_authority` right after** (13.6–17.6 s each, the first 27.6 s). `uhd_usrp_probe` right after the first failure exits 0 with no warning (`~/ezsdr-bench/probe-after-refpll.log`).
- Not the fix's (INFERRED: the failure is in UHD's clock-source call at the start of a process, before any stream; the fix changes only how streams stop and bursts join). Every test process re-initialises the X300 and re-selects its reference; the X300 had by then been running and re-initialised for about 8 hours.
- What it means for the Module: a Run whose Authority cannot select its source fails at assembly with UR-7's error, which is the specified behaviour (a Run on an unlocked reference must not start). Whether UR-7 should retry the selection once before failing is a question for the owner and Phase 8, not changed here.

## Session 2, part 4 — after Review N's fixes (`aaf1e00`)

The owner: "推奨で直して再レビューをしてください". The fixes are design-notes §12; then on the bench at `aaf1e00`, logs in `~/ezsdr-bench/s2-reviewN/`.

### The low rate and the long block — `hw_b8_cold_change_capture_low_rate`: pass

A Session at 390 625 S/s (200 MHz / 512, the lowest rate), a capture across a `cold` change, once to 400 000 S/s with the default block and once to 2 Msps with `block_len` 65 536 (168 ms a block). The test now asserts: the old clock's samples end at `e₁`, the new clock's begin at `e₂`, no gap, no `LATE_COMMAND`.

```
B8 cold 390625 → 400000 S/s, block_len None: capture asked at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 484981833 }; rate change submitted at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 493099547 }: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }
B8 cold: stats {"link_drops_seen":0,"rx_before_origin":2,"rx_blocks":225,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":448063,"tx_bursts":0,"tx_samples":0}
B8 cold: artifact rec_0 continuity [ContinuityMap { domain: ClockDomainId { node: NodeId(0), local: 4 }, channels: 1, valid: [[Segment { start: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 86079 }, len: 35559 }]], gaps: [], channel_gaps: [], first: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 86079 }, end: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 121638 } }, ContinuityMap { domain: ClockDomainId { node: NodeId(0), local: 5 }, channels: 1, valid: [[Segment { start: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 5 }, ticks: 0 }, len: 51316 }]], gaps: [], channel_gaps: [], first: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 5 }, ticks: 0 }, end: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 5 }, ticks: 51316 } }]
B8 cold 390625 → 2000000 S/s, block_len Some(65536): capture asked at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 484736279 }; rate change submitted at TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 492822188 }: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }
B8 cold: stats {"link_drops_seen":0,"rx_before_origin":2,"rx_blocks":19,"rx_errors":0,"rx_off_lattice":0,"rx_overflows":0,"rx_overlapping":0,"rx_samples":1143848,"tx_bursts":0,"tx_samples":0}
B8 cold: artifact rec_0 continuity [ContinuityMap { domain: ClockDomainId { node: NodeId(0), local: 4 }, channels: 1, valid: [[Segment { start: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 86020 }, len: 35747 }]], gaps: [], channel_gaps: [], first: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 86020 }, end: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, ticks: 121767 } }, ContinuityMap { domain: ClockDomainId { node: NodeId(0), local: 5 }, channels: 1, valid: [[Segment { start: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 5 }, ticks: 0 }, len: 211128 }]], gaps: [], channel_gaps: [], first: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 5 }, ticks: 0 }, end: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 5 }, ticks: 211128 } }]
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 27 filtered out; finished in 13.10s
```

- Both pass. The old stream's tail after the untimed stop is dropped (`rx_before_origin: 2` in each), not published.
- From `timing`: with the default block the untimed stop came 0.41 ms after `e₁` and the switch 49.2 ms before `e₂`; **with the 168 ms block, the untimed stop came 25.5 ms after `e₁`** and the switch 23.8 ms before `e₂`. Review N B1's fix bounds every receive request issued once the cut is known, but not the one already in progress when the change is booked, which can last a whole block. **Residual (INFERRED from this and the code, recorded as design-notes §12 R-1 for the owner and the re-review):** with a block longer than about the restart lead less the delivery, a change booked early in a receive call can still miss `e₂` — a `LATE_COMMAND` and UR-17's restart, reported, and no old sample on the new clock (`not_before`).

`hw_b8_cold_change_capture` (1 Msps → 2 Msps, the default block), also with the new assertions: pass; the untimed stop 1.85 ms after `e₁`, the switch 47.6 ms before `e₂`. Its first run failed for a reason of the test run, not the Module: the name filter also matched the new `…_low_rate`, so two tests opened the X300 at once in one process — UHD logged `[ERROR] [RFNOC::GRAPH::DETAIL] Attempting to reconnect output port 0/DDC#0:0`, the Session failed at `Arm`, and the process died of SIGSEGV (`hw_b8_cold_change_capture.two-in-parallel.log`). The bench script now passes `--exact`.

### A burst booked at a sent burst's end — `hw_b8_burst_at_a_sent_burst_s_end`: pass

```
B8 burst at a sent burst's end, trial 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; 7.63583 ms before A's end; TIME_ERROR []; async [{"channel":0,"code":"BurstAck","tick":415642001}]; bursts [{"actual_start":null,"blocks":15,"end":"eob","late_by":null,"requested_target":null,"samples":30000,"target":{"domain":{"local":4,"node":0},"ticks":41237},"wraps":1},{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":1000,"target":{"domain":{"local":4,"node":0},"ticks":71237},"wraps":1}]
B8 burst at a sent burst's end, trial 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; 7.89747 ms before A's end; TIME_ERROR []; async [{"channel":0,"code":"BurstAck","tick":415593401}]; bursts [{"actual_start":null,"blocks":15,"end":"eob","late_by":null,"requested_target":null,"samples":30000,"target":{"domain":{"local":4,"node":0},"ticks":41404},"wraps":1},{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":1000,"target":{"domain":{"local":4,"node":0},"ticks":71404},"wraps":1}]
B8 burst at a sent burst's end, trial 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; 7.570635 ms before A's end; TIME_ERROR []; async [{"channel":0,"code":"BurstAck","tick":415777201}]; bursts [{"actual_start":null,"blocks":15,"end":"eob","late_by":null,"requested_target":null,"samples":30000,"target":{"domain":{"local":4,"node":0},"ticks":41605},"wraps":1},{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":1000,"target":{"domain":{"local":4,"node":0},"ticks":71605},"wraps":1}]
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 27 filtered out; finished in 39.68s
```

- In all three trials the burst booked 7.6–7.9 ms before the first one's end (after its last buffer went out) is admitted, played and acknowledged (one `BurstAck` for the pair: one device burst), with no `TIME_ERROR`, no `TimeError`, no `Underflow`: Review N B3 closed on the bench.

### Preemption and the transmit end, again

```
B8 preempt lead 20 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []
B8 preempt lead 20 ms: asked 121717; bursts [{"actual_start":null,"blocks":117,"end":"eob","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402600037},"samples":116638,"target":{"domain":{"local":4,"node":0},"ticks":5079},"wraps":116},{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":100,"target":{"domain":{"local":4,"node":0},"ticks":121717},"wraps":1}]
B8 preempt lead 10 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(725000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(111492)}}]
B8 preempt lead 10 ms: asked 111492; bursts [{"actual_start":null,"blocks":208,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402395897},"samples":208000,"target":{"domain":{"local":4,"node":0},"ticks":5217},"wraps":208}]
B8 preempt lead 5 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(5500000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(106542)}}]
B8 preempt lead 5 ms: asked 106542; bursts [{"actual_start":null,"blocks":209,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402249328},"samples":209000,"target":{"domain":{"local":4,"node":0},"ticks":5042},"wraps":209}]
B8 preempt lead 3 ms: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(78000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(104371)}}]
B8 preempt lead 3 ms: asked 104371; bursts [{"actual_start":null,"blocks":209,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402292343},"samples":209000,"target":{"domain":{"local":4,"node":0},"ticks":5031},"wraps":209}]
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 27 filtered out; finished in 49.25s
B8 stop end, trial 0: Stop submitted at root 415600770; the loop heard the repeat until root 417686200 (+10.427 ms from the Stop); steady power 1.24e-4, after 7.60e-9; bursts [{"actual_start":null,"blocks":77,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402284508},"samples":77000,"target":{"domain":{"local":4,"node":0},"ticks":5029},"wraps":77}]
B8 stop end, trial 0: tx clocks [(401278800, 200)]
B8 stop end, trial 1: Stop submitted at root 415660978; the loop heard the repeat until root 418015800 (+11.774 ms from the Stop); steady power 1.24e-4, after 7.73e-9; bursts [{"actual_start":null,"blocks":78,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402412539},"samples":78000,"target":{"domain":{"local":4,"node":0},"ticks":5143},"wraps":78}]
B8 stop end, trial 1: tx clocks [(401384000, 200)]
B8 stop end, trial 2: Stop submitted at root 415723318; the loop heard the repeat until root 417812200 (+10.444 ms from the Stop); steady power 1.24e-4, after 8.37e-9; bursts [{"actual_start":null,"blocks":77,"end":"stop","late_by":null,"requested_target":{"domain":{"local":2,"node":0},"ticks":402401275},"samples":77000,"target":{"domain":{"local":4,"node":0},"ticks":5164},"wraps":77}]
B8 stop end, trial 2: tx clocks [(401368600, 200)]
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 27 filtered out; finished in 19.62s
```

- The 20 ms preemption is played as at part 3; 10, 5 and 3 ms are dropped by UR-21 as before.
- The transmission ends 10.4–11.8 ms after the Stop's submission (part 2: 10.3–11.4 ms).

### B1–B8, B9's long Runs, B7 (Python): pass

B1, B2, B3, B4, B5, B6 (delay 44 samples, the repeat back to back), B7 (Rust), B8 (`hw_b8_leads`, `hw_b8_stop_end`), `hw_b9_long_receive` (120 s), `hw_b9_transmit_only_session` (60 s): all pass, with no reference-PLL failure this time. B7 (Python), release server rebuilt from `aaf1e00`:

```
sample rate 20000000.0 S/s (coerced, a new SampleClock)
the transmit retune outside the RF envelope was refused: radio.rf_envelope: RM-19: radio: 2400000000 Hz is in no allowed band
y: correlation peak at sample 2952; z: at sample 864
rec_0: first sample {'domain': {'node': 0, 'local': 3}, 'ticks': 55446}
rec_1: first sample {'domain': {'node': 0, 'local': 3}, 'ticks': 179534}
asked: y at {'domain': {'node': 0, 'local': 2}, 'ticks': 411604875}, z at {'domain': {'node': 0, 'local': 2}, 'ticks': 436422504}
TIME_ERROR events: 0 (spike K6: none)
Manifest: /bench/ezsdr-runs/session-7-7/manifest.json
```

### Review O's O-B1 on the bench — `hw_b8_raw_empty_eob_gap`: **VERIFIED**

Review O inferred that a burst closed by an empty end-of-burst — as uhd-tx closes every burst since `aaf1e00`, a device lead before its end — ends one sample late on the device, because UHD sends an empty send as one zero sample (`tx_streamer_impl.hpp:266–276`), so that a burst one sample after it meets the X300's gap-0 drop. The owner: "推測を実機で確かめてください". On the `Device` itself at 1 Msps (1 GHz, 0 dB, amplitude 0.4, the 30 dB loopback): a 1 000-sample burst A, closed either by an empty end-of-burst 2 ms before its end or by end-of-burst on its last data buffer, then a 100-sample burst B with a timed start-of-burst 0, 1, 2, 3 or 5 samples after A's end, 4 times each:

```
[UHD printed 134 × "L"] B8 A closed by an empty end-of-burst 2 ms before its end, B at gap 0 samples: 4/4 late; ["BurstAck+TimeError", "BurstAck+TimeError", "BurstAck+TimeError", "BurstAck+TimeError"]
[UHD printed 134 × "L"] B8 A closed by an empty end-of-burst 2 ms before its end, B at gap 1 samples: 4/4 late; ["BurstAck+TimeError", "BurstAck+TimeError", "BurstAck+TimeError", "BurstAck+TimeError"]
B8 A closed by an empty end-of-burst 2 ms before its end, B at gap 2 samples: 0/4 late; ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 A closed by an empty end-of-burst 2 ms before its end, B at gap 3 samples: 0/4 late; ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 A closed by an empty end-of-burst 2 ms before its end, B at gap 5 samples: 0/4 late; ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
[UHD printed 128 × "L"] B8 A closed by end-of-burst on its last data buffer, B at gap 0 samples: 4/4 late; ["BurstAck+TimeError", "BurstAck+TimeError", "BurstAck+TimeError", "BurstAck+TimeError"]
B8 A closed by end-of-burst on its last data buffer, B at gap 1 samples: 0/4 late; ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 A closed by end-of-burst on its last data buffer, B at gap 2 samples: 0/4 late; ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 A closed by end-of-burst on its last data buffer, B at gap 3 samples: 0/4 late; ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
B8 A closed by end-of-burst on its last data buffer, B at gap 5 samples: 0/4 late; ["BurstAck", "BurstAck", "BurstAck", "BurstAck"]
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 28 filtered out; finished in 20.57s
```

| A closed by | gap 0 | gap 1 | gap 2 | gap 3 | gap 5 |
|---|---|---|---|---|---|
| an empty end-of-burst, 2 ms before its end (`aaf1e00`) | 4/4 late | **4/4 late** | 0/4 | 0/4 | 0/4 |
| end-of-burst on its last data buffer (`bc0db98`) | 4/4 late | 0/4 | 0/4 | 0/4 | 0/4 |

- **O-B1 is real:** the empty end-of-burst moves the device burst's end one sample later, so a burst at gap 1 — played when A's last data buffer carried end-of-burst — is now late and dropped. A regression of `aaf1e00`, open (Review O's recommendation, or another, is the owner's call).
- Gap 0 is late either way (as `hw_b8_raw_burst_gap`), which is why uhd-tx continues the device burst there.

## Session 2, part 5 — after Review O's fixes (`10ddb73`, `b3c94ba`)

The owner: "それでは二つとも推奨で直してください", and on O-B2: "O-B2で時間固定はサンプリングレートを変えてもこの時間でいいの？たとえば100kspsでも200Mspsでも大丈夫" — the bound is the old stream's block at its rate, not a fixed time; the question showed the packet term was missing (design-notes §13). Logs in `~/ezsdr-bench/s2-reviewO/`.

### O-B2 at the rates' extremes — `hw_b8_cold_change_timing_at_the_extremes`: pass

A receive `cold` change booked at four phases of the receive call in progress, at 200 Msps → 100 Msps (the highest rate), 390 625 → 400 000 S/s with `block_len` 100 (the packet, 1 996 samples, longer than the block) and 390 625 S/s → 2 Msps with `block_len` 65 536 (168 ms a block). No capture (200 Msps of `cf32` is 1.6 GB/s); the Manifest's timing. Each asserts `e₁` past the call in progress, the switch before `e₂`, no `LATE_COMMAND` (`stats` cut from the lines below):

```
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(-0.0426) ms after e₁; switch 49.675 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.050135) ms after e₁; switch 49.774 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.006885) ms after e₁; switch 49.700 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 3: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.0311) ms after e₁; switch 49.646 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.112 ms (floor 58.110 ms); untimed stop Some(1.28296) ms after e₁; switch 48.274 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.111 ms (floor 58.110 ms); untimed stop Some(0.571795) ms after e₁; switch 48.851 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.112 ms (floor 58.110 ms); untimed stop Some(3.349935) ms after e₁; switch 46.340 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 3: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.111 ms (floor 58.110 ms); untimed stop Some(2.91755) ms after e₁; switch 46.701 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.773 ms (floor 220.772 ms); untimed stop Some(4.338895) ms after e₁; switch 44.834 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.773 ms (floor 220.772 ms); untimed stop Some(3.48607) ms after e₁; switch 46.128 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.774 ms (floor 220.772 ms); untimed stop Some(2.471775) ms after e₁; switch 46.873 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 3: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.775 ms (floor 220.772 ms); untimed stop Some(0.65763) ms after e₁; switch 48.830 ms before e₂; LATE_COMMAND 0
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 88.68s
```

- **All twelve: `e₁` at its floor (53.0, 58.1, 220.8 ms after the booking), the switch 44.8–49.8 ms before `e₂`, no `LATE_COMMAND`.** The untimed stop came −0.04…+4.3 ms from `e₁`; the negative values at 200 Msps are the Authority's host-derived time running ~0.1 ms behind the device's (part 2's 60 s drift), the stop issued once the samples up to `e₁` had arrived.
- `rx_packet_samples()` read 1 996 at every rate (UHD's `max_num_samps` at MTU 9000).
- The fixed 50 ms and 3 ms hold at both ends on this bench: the largest stop after `e₁` was 4.3 ms, at the lowest rate with the longest block.

### O-B1 — `hw_b8_burst_one_sample_after_a_burst`: pass

```
B8 burst one sample after a burst, booked late false: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"channel":0,"code":"BurstAck","tick":415556401},{"channel":0,"code":"BurstAck","tick":415756601}]; bursts [{"actual_start":null,"blocks":15,"end":"eob","late_by":null,"requested_target":null,"samples":30000,"target":{"domain":{"local":4,"node":0},"ticks":41174},"wraps":1},{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":1000,"target":{"domain":{"local":4,"node":0},"ticks":71175},"wraps":1}]
B8 burst one sample after a burst, booked late true: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"channel":0,"code":"BurstAck","tick":415458801},{"channel":0,"code":"BurstAck","tick":415659001}]; bursts [{"actual_start":null,"blocks":15,"end":"eob","late_by":null,"requested_target":null,"samples":30000,"target":{"domain":{"local":4,"node":0},"ticks":41448},"wraps":1},{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":1000,"target":{"domain":{"local":4,"node":0},"ticks":71449},"wraps":1}]
B8 burst one sample after a burst, booked late false: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"channel":0,"code":"BurstAck","tick":415489601},{"channel":0,"code":"BurstAck","tick":415689801}]; bursts [{"actual_start":null,"blocks":15,"end":"eob","late_by":null,"requested_target":null,"samples":30000,"target":{"domain":{"local":4,"node":0},"ticks":41382},"wraps":1},{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":1000,"target":{"domain":{"local":4,"node":0},"ticks":71383},"wraps":1}]
B8 burst one sample after a burst, booked late true: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"channel":0,"code":"BurstAck","tick":415636201},{"channel":0,"code":"BurstAck","tick":415836401}]; bursts [{"actual_start":null,"blocks":15,"end":"eob","late_by":null,"requested_target":null,"samples":30000,"target":{"domain":{"local":4,"node":0},"ticks":41512},"wraps":1},{"actual_start":null,"blocks":1,"end":"eob","late_by":null,"requested_target":null,"samples":1000,"target":{"domain":{"local":4,"node":0},"ticks":71513},"wraps":1}]
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 31.21s
```

- A burst at a 30 000-sample burst's end + 1 sample, booked before and after the first's last buffer went out, twice each: played, no `TIME_ERROR`, no `TimeError`, no `Underflow` — where `aaf1e00`'s empty end-of-burst made it late (part 4: 4 of 4).

### The rest, again: pass

`hw_b8_burst_at_a_sent_burst_s_end`, `hw_b8_cold_change_capture`, `hw_b8_cold_change_capture_low_rate`, B1–B8 (B6: delay 44 samples), `hw_b8_preemption` (20 ms played), `hw_b8_stop_end` (the transmission ends 10.1–11.6 ms after the Stop's submission), `hw_b9_long_receive`, `hw_b9_transmit_only_session`: all pass, no reference-PLL failure. B7 (Python), release server rebuilt from `b3c94ba`:

```
sample rate 20000000.0 S/s (coerced, a new SampleClock)
the transmit retune outside the RF envelope was refused: radio.rf_envelope: RM-19: radio: 2400000000 Hz is in no allowed band
y: correlation peak at sample 12934; z: at sample 13052
rec_0: first sample {'domain': {'node': 0, 'local': 3}, 'ticks': 55865}
rec_1: first sample {'domain': {'node': 0, 'local': 3}, 'ticks': 179747}
asked: y at {'domain': {'node': 0, 'local': 2}, 'ticks': 411734732}, z at {'domain': {'node': 0, 'local': 2}, 'ticks': 436511002}
TIME_ERROR events: 0 (spike K6: none)
Manifest: /bench/ezsdr-runs/session-7-9/manifest.json
```


## Session 2, part 6 — after Review P's fixes (`ea63b3a`)

The owner: "推奨通りにしてください" (design-notes §14). `tx.rs` changed (a send cut short, the held sample), so every stage again, logs in `~/ezsdr-bench/s2-reviewP/`; the release server rebuilt from `ea63b3a` for B7 (Python). My first attempt at this pass ran no Rust stage (a wrong path in my runner script); while clearing it I deleted, without looking first, the 17 root-level `~/ezsdr-bench/hw_*.log` that its `mv` had swept in. Each was the copy of the last pass's log — every pass ran `hw.sh`, which writes the root log, then `cp`'d it into its `s2-*` directory, and part 5 ran all 17 — so the same logs remain in `~/ezsdr-bench/s2-reviewO/`.

### Two failures that are not this change's

**B2 — `hw_b2_authority`: the test passed, then the process aborted at exit** (first of the pass; never seen before in about ten B2 runs, `s2-fixed`'s eight loops included):

```
B2 anchor drift over 10.001079333s: 1911 ticks
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 23.05s
double free or corruption (out)
error: test failed, to rerun pass `-p ezsdr-radio-uhd --test hardware`
Caused by:
  process didn't exit successfully: `/cargo-target/release/deps/hardware-4fa4ba8ce0a41e45 hw_b2_authority --exact --ignored --nocapture --quiet` (signal: 6, SIGABRT: process abort signal)
```

Eight reruns (`hw_b2_authority.loop1.log` … `loop8.log`): all pass, none aborts. This commit does not touch `uhd.rs` or `authority.rs`. INFERRED cause, recorded as a finding (design-notes §14, D-1) and not fixed: `DeviceAuthority`'s `drop` only sets its stop flag and does not join its `uhd-clock` thread, which every 100 ms (`REANCHOR`) upgrades a weak reference to the device time and reads the device; when the test returns while that thread holds the device, the process can exit with it still in UHD, or free the `uhd_usrp` on that thread while UHD's static destructors run.

**B8 leads — `hw_b8_leads`: the reference PLL did not lock** (UR-7; part 3's finding), twice: on its 4th device open, and on the rerun's 3rd. The second rerun passed:

```
called `Result::unwrap()` on an `Err` value: "UR-7: uhd_usrp_set_clock_source: UHD error 44: RuntimeError: Reference Clock PLL failed to lock to internal source."
called `Result::unwrap()` on an `Err` value: "UR-7: uhd_usrp_set_clock_source: UHD error 44: RuntimeError: Reference Clock PLL failed to lock to internal source."
```

In this pass 2 of 56 device opens failed this way (part 3: 2 of about 24).

### B8 leads — second rerun: pass

```
B8 lead 10000 µs: TIME_ERROR []
B8 lead 5000 µs: TIME_ERROR []
B8 lead 3000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(40000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(3141)}}]
B8 lead 2000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(1292000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(2031)}}]
B8 lead 1500 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(1543000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(1567)}}]
B8 lead 1000 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(2232000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(1070)}}]
B8 lead 500 µs: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(2742000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(522)}}]
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 53.92s
```

### The rest: pass

```
B6: transmit-to-receive delay 44 samples
B6 repeat: 3 whole periods from sample 44 (offset, gain, phase °): [(44, 0.01545647, 27.83225), (1044, 0.0154486215, 27.652273), (2044, 0.0154524455, 27.582119)]
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 7.55s
B8 burst at a sent burst's end, trial 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; 7.640935 ms before A's end; TIME_ERROR [ …
B8 burst at a sent burst's end, trial 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; 7.83131 ms before A's end; TIME_ERROR [] …
B8 burst at a sent burst's end, trial 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; 7.68624 ms before A's end; TIME_ERROR [] …
B8 burst one sample after a burst, booked late false: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"cha …
B8 burst one sample after a burst, booked late true: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"chan …
B8 burst one sample after a burst, booked late false: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"cha …
B8 burst one sample after a burst, booked late true: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"chan …
```

```
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.07541) ms after e₁; switch 49.664 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(-0.063695) ms after e₁; switch 49.687 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(-0.01191) ms after e₁; switch 49.890 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 3: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.052155) ms after e₁; switch 49.796 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.110 ms (floor 58.110 ms); untimed stop Some(1.232675) ms after e₁; switch 48.512 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.111 ms (floor 58.110 ms); untimed stop Some(0.551845) ms after e₁; switch 48.924 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.111 ms (floor 58.110 ms); untimed stop Some(4.361855) ms after e₁; switch 45.363 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 3: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.112 ms (floor 58.110 ms); untimed stop Some(2.76301) ms after e₁; switch 46.951 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.773 ms (floor 220.772 ms); untimed stop Some(4.105195) ms after e₁; switch 45.700 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.774 ms (floor 220.772 ms); untimed stop Some(3.20607) ms after e₁; switch 46.377 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.773 ms (floor 220.772 ms); untimed stop Some(1.244475) ms after e₁; switch 48.147 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 3: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.772 ms (floor 220.772 ms); untimed stop Some(1.2458) ms after e₁; switch 48.033 ms before e₂; LATE_COMMAND 0
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 82.62s
```

- B1, B3, B4, B5 (restart gap 455.7 ms at 10 Msps), B6 (delay 44 samples; the repeat's three periods at 44, 1 044, 2 044), B7 (Rust), B8 preemption (20 ms played), B8 stop end, `hw_b8_burst_at_a_sent_burst_s_end` (booked 7.6–7.8 ms before A's end, played, no `TIME_ERROR`), `hw_b8_burst_one_sample_after_a_burst` (4 of 4 played), both `cold_change_capture` tests, `hw_b9_long_receive` (`DEVICE_LOST []`, overflows 0) and `hw_b9_transmit_only_session` pass.
- O-B2 at the extremes again: `e₁` at its floor, the switch 45.4–49.9 ms before `e₂`, no `LATE_COMMAND`; the untimed stop at most 4.4 ms after `e₁`.
- NB-1's path (a send cut short) cannot be produced on the bench: the X300 took every send. The held sample's order (TG-1) is one sample in 31 000, below what the 30 dB loop's correlation resolves; both are pinned on `FakeDevice` (design-notes §14).

B7 (Python), `minimal.py` and `bench_loopback.py`:

```
captured 100000 samples, mean power 0.0001
Manifest: /bench/ezsdr-runs/session-7-12/manifest.json (3 logged calls)
sample rate 20000000.0 S/s (coerced, a new SampleClock)
the transmit retune outside the RF envelope was refused: radio.rf_envelope: RM-19: radio: 2400000000 Hz is in no allowed band
y: correlation peak at sample 1881; z: at sample 10009
rec_0: first sample {'domain': {'node': 0, 'local': 3}, 'ticks': 54539}
rec_1: first sample {'domain': {'node': 0, 'local': 3}, 'ticks': 179411}
asked: y at {'domain': {'node': 0, 'local': 2}, 'ticks': 411648095}, z at {'domain': {'node': 0, 'local': 2}, 'ticks': 436622508}
TIME_ERROR events: 0 (spike K6: none)
Manifest: /bench/ezsdr-runs/session-7-13/manifest.json
```


## Session 2, part 7 — after Review Q's fixes (`bd8151c`); the reference PLL

The owner: "推奨ですすめてください" (design-notes §15). Every stage again, then `hw_b2_authority` twenty times for D-1; logs in `~/ezsdr-bench/s2-reviewQ/`, the release server rebuilt from `bd8151c`. The runner's summary, as printed:

```
hw_b1_probe exit=0
hw_b2_authority exit=0
hw_b3_receive_at_t0 exit=0
hw_b4_capture_at_a_sample_index exit=0
hw_b5_overflow exit=0
hw_b6_txrx_and_repeat exit=0
hw_b7_session_loopback exit=0
hw_b8_leads exit=0
hw_b8_preemption exit=0
hw_b8_stop_end exit=0
hw_b8_burst_at_a_sent_burst_s_end exit=0
hw_b8_burst_one_sample_after_a_burst exit=0
hw_b8_cold_change_capture exit=0
hw_b8_cold_change_capture_low_rate exit=0
hw_b8_cold_change_timing_at_the_extremes exit=101
hw_b9_long_receive exit=0
hw_b9_transmit_only_session exit=0
server build exit=0
b7 minimal exit=0
b7 bench_loopback exit=0
b2 loop1 exit=0 abort=0
b2 loop2 exit=0 abort=0
b2 loop3 exit=0 abort=0
b2 loop4 exit=0 abort=0
b2 loop5 exit=101 abort=0
b2 loop6 exit=0 abort=0
b2 loop7 exit=0 abort=0
b2 loop8 exit=0 abort=0
b2 loop9 exit=0 abort=0
b2 loop10 exit=0 abort=0
b2 loop11 exit=0 abort=0
b2 loop12 exit=0 abort=0
b2 loop13 exit=0 abort=0
b2 loop14 exit=0 abort=0
b2 loop15 exit=0 abort=0
b2 loop16 exit=0 abort=0
b2 loop17 exit=0 abort=0
b2 loop18 exit=0 abort=0
b2 loop19 exit=0 abort=0
b2 loop20 exit=0 abort=0
```

### D-1: no abort in 21 B2 runs

`hw_b2_authority` passed in the pass and in 19 of the 20 loops; loop 5 failed on the reference PLL (below) before it ran. No run aborted at exit (`double free`, `SIGABRT`: 0 in every log). Part 6 saw one abort in about twenty runs, so twenty clean runs alone show little; the fix rests on the fake tests that failed on the old code (design-notes §15).

### The reference PLL: 4 failures in 66 opens, the extremes test not passed

`hw_b8_cold_change_timing_at_the_extremes` failed three times running, each on a device open that did not lock (on its 6th, its 3rd, and its first open); B2's loop 5 failed the same way:

```
called `Result::unwrap()` on an `Err` value: "UR-7: uhd_usrp_set_clock_source: UHD error 44: RuntimeError: Reference Clock PLL failed to lock to internal source."
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.077325) ms after e₁; switch 49.562 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.11933) ms after e₁; switch 49.488 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.04455) ms after e₁; switch 49.705 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 3: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.074065) ms after e₁; switch 49.794 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.112 ms (floor 58.110 ms); untimed stop Some(1.355245) ms after e₁; switch 48.248 ms before e₂; LATE_COMMAND 0
called `Result::unwrap()` on an `Err` value: "UR-7: uhd_usrp_set_clock_source: UHD error 44: RuntimeError: Reference Clock PLL failed to lock to internal source."
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 30 filtered out; finished in 75.35s
called `Result::unwrap()` on an `Err` value: "UR-7: uhd_usrp_set_clock_source: UHD error 44: RuntimeError: Reference Clock PLL failed to lock to internal source."
called `Result::unwrap()` on an `Err` value: "UR-7: uhd_usrp_set_clock_source: UHD error 44: RuntimeError: Reference Clock PLL failed to lock to internal source."
```

The five phases it reached before failing are as in parts 5 and 6 (`e₁` at its floor, the switch 48.2–49.8 ms before `e₂`, no `LATE_COMMAND`).

**The device's state has changed since the handoff.** B1 reads the motherboard's `ref_locked` sensor right after the open, before any source is set:

| Pass | B1 `ref_locked` | Device opens | PLL lock failures |
|---|---|---|---|
| part 3 (`s2-fixed`) | `Some(true)` | 39 | 3 |
| part 4 (`s2-reviewN`) | `Some(true)` | 34 | 0 |
| part 5 (`s2-reviewO`) | `Some(false)` | 48 | 0 |
| part 6 (`s2-reviewP`) | `Some(false)` | 63 | 2 |
| part 7 (`s2-reviewQ`) | `Some(false)` | 66 | 4 |

The cause is not known (INFERRED candidates: the X300's temperature after a day of runs, its reference oscillator or PLL). Nothing in the code reads or sets the reference differently since part 4. The hardware runs stop here, as the session's instructions say for a device in a state other than the handoff's; the owner is asked.

### The rest: pass

```
B6: transmit-to-receive delay 44 samples
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 9.57s
B8 burst at a sent burst's end, trial 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; 7.87021 ms before A's end; TIME_ERROR [] …
B8 burst at a sent burst's end, trial 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; 7.64496 ms before A's end; TIME_ERROR [] …
B8 burst at a sent burst's end, trial 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; 7.64791 ms before A's end; TIME_ERROR [] …
B8 burst one sample after a burst, booked late false: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"cha …
B8 burst one sample after a burst, booked late true: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"chan …
B8 burst one sample after a burst, booked late false: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"cha …
B8 burst one sample after a burst, booked late true: Admitted { coercions: [], warnings: [], dispatched: [ActionId(3)] }; TIME_ERROR []; async [{"chan …
B5 gap OverflowRestart at receive sample 11108408 for 4684057 samples (468.4057 ms at 10 Msps), lost Some(4684057)
B9 long receive: Ok(()); termination Stopped { cause: Client }; DEVICE_LOST []; overflows 0; stats {"link_drops_seen":0,"rx_before_origin":0,"rx_blocks":600011,"rx_errors":0,"rx_off_lattice":0,"rx_ove …
```

B1, B3, B4, B7 (Rust), B8 leads, preemption, stop end, both `cold_change_capture` tests and `hw_b9_transmit_only_session` pass. B7 (Python):

```
the transmit retune outside the RF envelope was refused: radio.rf_envelope: RM-19: radio: 2400000000 Hz is in no allowed band
y: correlation peak at sample 2635; z: at sample 1301
rec_0: first sample {'domain': {'node': 0, 'local': 3}, 'ticks': 54700}
rec_1: first sample {'domain': {'node': 0, 'local': 3}, 'ticks': 180034}
TIME_ERROR events: 0 (spike K6: none)
```


## Session 2, part 8 — after the X300's power cycle (`7793fc7`)

The owner powered the X300 off after part 7, then on: "X300の電源を入れました．抜線もできます". `7793fc7` changes only the fake and its tests since part 7 (Review R's NB-R3, TG-R1). Logs in `~/ezsdr-bench/s2-poweron/`.

**B1 right after the power cycle** (its log was then overwritten by the pass's B1 below — my runner moves each log into the same directory; the lines as they were printed):

```
B1 ref_locked: Ok(Some(true))
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 3.55s
```

Then every stage, and `hw_b2_authority` twenty times; the runner's summary:

```
hw_b1_probe exit=0
hw_b2_authority exit=0
hw_b3_receive_at_t0 exit=0
hw_b4_capture_at_a_sample_index exit=0
hw_b5_overflow exit=0
hw_b6_txrx_and_repeat exit=0
hw_b7_session_loopback exit=0
hw_b8_leads exit=0
hw_b8_preemption exit=0
hw_b8_stop_end exit=0
hw_b8_burst_at_a_sent_burst_s_end exit=0
hw_b8_burst_one_sample_after_a_burst exit=0
hw_b8_cold_change_capture exit=0
hw_b8_cold_change_capture_low_rate exit=0
hw_b8_cold_change_timing_at_the_extremes exit=0
hw_b9_long_receive exit=0
hw_b9_transmit_only_session exit=0
server build exit=0
b7 minimal exit=0
b7 bench_loopback exit=0
b2 loop1 exit=0 abort=0
b2 loop2 exit=0 abort=0
b2 loop3 exit=101 abort=0
b2 loop4 exit=0 abort=0
b2 loop5 exit=0 abort=0
b2 loop6 exit=0 abort=0
b2 loop7 exit=0 abort=0
b2 loop8 exit=0 abort=0
b2 loop9 exit=101 abort=0
b2 loop10 exit=0 abort=0
b2 loop11 exit=0 abort=0
b2 loop12 exit=0 abort=0
b2 loop13 exit=0 abort=0
b2 loop14 exit=101 abort=0
b2 loop15 exit=0 abort=0
b2 loop16 exit=0 abort=0
b2 loop17 exit=0 abort=0
b2 loop18 exit=0 abort=0
b2 loop19 exit=0 abort=0
b2 loop20 exit=0 abort=0
```

- **Every stage passes**, `hw_b8_cold_change_timing_at_the_extremes` included (part 7 could not finish it):

```
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.07041) ms after e₁; switch 49.577 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.026505) ms after e₁; switch 49.802 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(-0.096635) ms after e₁; switch 49.693 ms before e₂; LATE_COMMAND 0
B8 cold 200000000 → 100000000 S/s, block_len None, packet 1996, phase 3: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 53.010 ms (floor 53.010 ms); untimed stop Some(0.086745) ms after e₁; switch 49.546 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.110 ms (floor 58.110 ms); untimed stop Some(2.358605) ms after e₁; switch 47.261 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.112 ms (floor 58.110 ms); untimed stop Some(4.974255) ms after e₁; switch 44.626 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.111 ms (floor 58.110 ms); untimed stop Some(4.15285) ms after e₁; switch 45.538 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 400000 S/s, block_len Some(100), packet 1996, phase 3: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 58.112 ms (floor 58.110 ms); untimed stop Some(2.78481) ms after e₁; switch 46.803 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.772 ms (floor 220.772 ms); untimed stop Some(3.79542) ms after e₁; switch 45.690 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 1: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.774 ms (floor 220.772 ms); untimed stop Some(3.09304) ms after e₁; switch 46.530 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 2: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.773 ms (floor 220.772 ms); untimed stop Some(1.41314) ms after e₁; switch 48.031 ms before e₂; LATE_COMMAND 0
B8 cold 390625 → 2000000 S/s, block_len Some(65536), packet 1996, phase 3: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; e₁ − booking 220.774 ms (floor 220.772 ms); untimed stop Some(1.368485) ms after e₁; switch 48.082 ms before e₂; LATE_COMMAND 0
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 100.67s
```

- **The reference PLL still fails to lock now and then:** B2's loops 3, 9 and 14, 3 of 68 opens in the pass:

```
called `Result::unwrap()` on an `Err` value: "UR-7: uhd_usrp_set_clock_source: UHD error 44: RuntimeError: Reference Clock PLL failed to lock to internal source."
```

- **`ref_locked` is not a sign of it:** it read `true` at the first open after the power cycle and `false` at the pass's B1, minutes later; the failures came either way. Part 7's table is therefore no evidence of a degrading device; what remains is a lock failure on about 4 % of opens since part 6 (2 of 63, 4 of 66, 3 of 68), against 3 of 39 in part 3 and 0 in parts 4–5. Why `ref_locked` reads `false` is not known.
- **D-1:** no abort at exit in 18 B2 runs that ran (or in any log of the pass).
- B6 delay 44 samples; B7 (Python): TIME_ERROR events: 0 (spike K6: none).


## Session 2, part 9 — B9, the unplug (`fdea39d`)

The owner: "準備OK"; then, on my "今抜いてください", unplugged the 10 GbE cable **at the X300's end** ("X300側を抜きました"). The host's NIC (`enp2s0f0np0`) carrier was logged every 10 ms with the host's UTC (`~/ezsdr-bench/s2-b9/carrier.log`):

```
1790833054.634383214 carrier=1 (start)
1790833081.848528081 carrier=0
1790833117.715439794 carrier=1
```

### Receive Run — `hw_b9_unplug`: **the process aborted** (B9 fails)

Started at UTC 1790833057.830 (05:37:37); the carrier dropped at 05:38:01.849; the log was last written at 05:38:07.919 (its file time), about 6.1 s after the unplug. The whole log, as printed:

```

running 1 test
[INFO] [UHD] linux; GNU C++ version 15.2.0; Boost_109000; UHD_4.10.0.0-0-unknown
[INFO] [X300] X300 initialization sequence...
[INFO] [X300] Maximum frame size: 8000 bytes.
[INFO] [X300] Radio 1x clock: 200 MHz
B9: unplug the cable now (receive Run; up to 300 s); started at UTC 1790833057.830
[ERROR] [0/Radio#0::CTRLEP] Control operation timed out waiting for ACK. Request sent: ctrl_payload{dst_port:4, src_port:4, seq_num:18, timestamp:<not present>, is_ack:false, src_epid:1, address:0x00004, byte_enable:0xf, op_code:2, status:0, num_data:1 data[0]:0x00000000}

[ERROR] [X300] 192.168.40.36: x300 fw communication failure #1
EnvironmentError: IOError: x300 fw poke32 - reply timed out
[ERROR] [0/Radio#0::CTRLEP] Control operation timed out waiting for ACK. Request sent: ctrl_payload{dst_port:4, src_port:4, seq_num:19, timestamp:<not present>, is_ack:false, src_epid:1, address:0x00004, byte_enable:0xf, op_code:2, status:0, num_data:1 data[0]:0x00000000}

[ERROR] [X300] 192.168.40.36: x300 fw communication failure #2
EnvironmentError: IOError: x300 fw poke32 - reply timed out
[ERROR] [0/Radio#0::CTRLEP] Control operation timed out waiting for ACK. Request sent: ctrl_payload{dst_port:4, src_port:4, seq_num:20, timestamp:<not present>, is_ack:false, src_epid:1, address:0x00004, byte_enable:0xf, op_code:2, status:0, num_data:1 data[0]:0x00000000}

[ERROR] [X300] 192.168.40.36: x300 fw communication failure #3
EnvironmentError: IOError: x300 fw poke32 - reply timed out
[ERROR] [UHD] An unexpected exception was caught in a task loop.The task loop will now exit, things may not work.EnvironmentError: IOError: 192.168.40.36: x300 fw communication failure #3
EnvironmentError: IOError: x300 fw poke32 - reply timed out
[ERROR] [0/Radio#0::CTRLEP] Control operation timed out waiting for ACK. Request sent: ctrl_payload{dst_port:4, src_port:4, seq_num:21, timestamp:<not present>, is_ack:false, src_epid:1, address:0x00004, byte_enable:0xf, op_code:2, status:0, num_data:1 data[0]:0x00000000}

[ERROR] [0/Radio#0::CTRLEP] Control operation timed out waiting for ACK. Request sent: ctrl_payload{dst_port:4, src_port:4, seq_num:22, timestamp:<not present>, is_ack:false, src_epid:1, address:0x00004, byte_enable:0xf, op_code:2, status:0, num_data:1 data[0]:0x00000000}

[ERROR] [X300] 192.168.40.36: x300 fw communication failure #1
EnvironmentError: IOError: x300 fw poke32 - reply timed out
[WARNING] [UHD] Exception caught in safe-call.
  in ~dboard_manager_impl
  at /root/tmp/uhd-4.10.0.0/host/lib/usrp/dboard_manager.cpp:506
set_nice_dboard_if() -> send: Network is unreachable [system:101]
[WARNING] [UHD] Exception caught in safe-call.
  in ~max287x
  at /root/tmp/uhd-4.10.0.0/host/lib/include/uhdlib/usrp/common/max287x.hpp:712
shutdown() -> EnvironmentError: IOError: send error on socket: Network is unreachable
[WARNING] [UHD] Exception caught in safe-call.
  in ~max287x
  at /root/tmp/uhd-4.10.0.0/host/lib/include/uhdlib/usrp/common/max287x.hpp:712
shutdown() -> EnvironmentError: IOError: send error on socket: Network is unreachable
[WARNING] [UHD] Exception caught in safe-call.
  in ~max287x
  at /root/tmp/uhd-4.10.0.0/host/lib/include/uhdlib/usrp/common/max287x.hpp:712
shutdown() -> EnvironmentError: IOError: send error on socket: Network is unreachable
[WARNING] [UHD] Exception caught in safe-call.
  in ~max287x
  at /root/tmp/uhd-4.10.0.0/host/lib/include/uhdlib/usrp/common/max287x.hpp:712
shutdown() -> EnvironmentError: IOError: send error on socket: Network is unreachable
[WARNING] [UHD] Exception caught in safe-call.
  in ~obx_cpld_ctrl
  at /root/tmp/uhd-4.10.0.0/host/lib/usrp/dboard/obx/obx_cpld_ctrl.cpp:36
_tx_value = 0; _rx_value = 0; write(); -> EnvironmentError: IOError: send error on socket: Network is unreachable
[WARNING] [UHD] Exception caught in safe-call.
  in ~obx_gpio_ctrl
  at /root/tmp/uhd-4.10.0.0/host/lib/usrp/dboard/obx/obx_gpio_ctrl.cpp:67
set_field(TX_GAIN, 0); set_field(CPLD_RST_N, 0); set_field(RX2_EN_N, 0); set_field(TX_EN_N, 1); set_field(RX_EN_N, 1); set_field(SPI_ADDR, 0x7); set_field(RX_GAIN, 0); set_field(TXLO1_SYNC, 0); set_field(TXLO2_SYNC, 0); set_field(RXLO1_SYNC, 0); set_field(RXLO1_SYNC, 0); write(); -> EnvironmentError: IOError: send error on socket: Network is unreachable
terminate called after throwing an instance of 'uhd::io_error'
  what():  EnvironmentError: IOError: send error on socket: Network is unreachable
error: test failed, to rerun pass `-p ezsdr-radio-uhd --test hardware`

Caused by:
  process didn't exit successfully: `/cargo-target/release/deps/hardware-4fa4ba8ce0a41e45 hw_b9_unplug --exact --ignored --nocapture --quiet` (signal: 6, SIGABRT: process abort signal)
```

- **Expected** (bench.md B9): `DEVICE_LOST` within about 1 s, the Run `Stopped { policy { DEVICE_LOST } }`, its Manifest written.
- **Observed:** UHD's control requests timed out (`x300 fw communication failure #1…#3`), UHD's own task loop exited on the error, and while the device was being torn down — the `safe-call` warnings come from destructors (`~max287x`, `~obx_cpld_ctrl`, `~obx_gpio_ctrl`) — a `uhd::io_error` escaped and the C++ runtime terminated the process (SIGABRT). None of the test's result lines printed, so it died before `finish` returned; whether the Run saw `DEVICE_LOST` and wrote its Manifest is unknown (the Run's directory was the container's and went with it).
- Where the exception escaped from is not known: a destructor that throws ends the process whatever catches it (C++ destructors are `noexcept`), so UHD's C API's own `try` cannot stop it. INFERRED from the log's order; a backtrace would need another unplug under a debugger.
- Recorded as design-notes §17 F4, for the owner. The transmit-only case and B3–B7 again have not been run.

The owner replugged the cable at the X300 ("X300側を挿し直しました"); the carrier came back at UTC 1790833117.715 (05:38:37.7), 35.9 s after it dropped. B1 then opened the device and passed (`~/ezsdr-bench/s2-b9/hw_b1_probe.after-replug.log`: OBX found, the time advancing 20.03–20.04 M ticks per 100 ms, `ref_locked` `Some(false)`).


### Receive Run again, under gdb — `hw_b9_unplug`: the abort's stack

The owner: "推奨で．gdbでもう一回やりましょう". The same test, its binary run under `gdb -batch` (cargo's runner; `catch signal SIGABRT`, then every thread's stack), the cable pulled at the X300 ("X300側を抜き，30秒待機して挿し直しました"): started at UTC 1790833372.179, the carrier dropped at 1790833398.062 and came back at 1790833433.153 (35.1 s). The process aborted as before (`terminate called after throwing an instance of 'uhd::io_error'`, the same destructor warnings). The aborting thread's stack, as gdb printed it (the whole log: `~/ezsdr-bench/s2-b9/hw_b9_unplug.gdb.log`; the other threads were UHD's logger, a control endpoint's receive worker, and the test harness's main thread, all waiting):

```
Thread 2 (Thread 0x7ffff5c6a6c0 (LWP 33) "hw_b9_unplug"):
#0  __pthread_kill_implementation (threadid=<optimized out>, signo=6, no_tid=0) at ./nptl/pthread_kill.c:44
#1  __pthread_kill_internal (threadid=<optimized out>, signo=6) at ./nptl/pthread_kill.c:89
#2  __GI___pthread_kill (threadid=<optimized out>, signo=signo@entry=6) at ./nptl/pthread_kill.c:100
#3  0x00007ffff68c8b7e in __GI_raise (sig=sig@entry=6) at ../sysdeps/posix/raise.c:26
#4  0x00007ffff68ab8ec in __GI_abort () at ./stdlib/abort.c:77
#5  0x00007ffff5db3275 in ?? () from /usr/lib/x86_64-linux-gnu/libstdc++.so.6
#6  0x00007ffff5dca64a in ?? () from /usr/lib/x86_64-linux-gnu/libstdc++.so.6
#7  0x00007ffff5db2af4 in __cxa_call_terminate () from /usr/lib/x86_64-linux-gnu/libstdc++.so.6
#8  0x00007ffff5dc9b2c in __gxx_personality_v0 () from /usr/lib/x86_64-linux-gnu/libstdc++.so.6
#9  0x00007ffff6bec592 in ?? () from /usr/lib/x86_64-linux-gnu/libgcc_s.so.1
#10 0x00007ffff6bed0db in _Unwind_Resume () from /usr/lib/x86_64-linux-gnu/libgcc_s.so.1
#11 0x00007ffff71e5a72 in ctrlport_endpoint_impl::poke32(unsigned int, unsigned int, uhd::time_spec_t, bool) () from /usr/local/lib/libuhd.so.4.10.0
#12 0x00007ffff798dc11 in x300_radio_control_impl::deinit() () from /usr/local/lib/libuhd.so.4.10.0
#13 0x00007ffff71c8797 in uhd::rfnoc::noc_block_base::shutdown() () from /usr/local/lib/libuhd.so.4.10.0
#14 0x00007ffff7178488 in uhd::rfnoc::detail::block_container_t::shutdown() () from /usr/local/lib/libuhd.so.4.10.0
#15 0x00007ffff71f6824 in rfnoc_graph_impl::~rfnoc_graph_impl() () from /usr/local/lib/libuhd.so.4.10.0
#16 0x00007ffff70fa8d7 in std::_Sp_counted_base<(__gnu_cxx::_Lock_policy)2>::_M_release_last_use_cold() () from /usr/local/lib/libuhd.so.4.10.0
#17 0x00007ffff73737f5 in multi_usrp_rfnoc::~multi_usrp_rfnoc() () from /usr/local/lib/libuhd.so.4.10.0
#18 0x00007ffff70fa8d7 in std::_Sp_counted_base<(__gnu_cxx::_Lock_policy)2>::_M_release_last_use_cold() () from /usr/local/lib/libuhd.so.4.10.0
#19 0x00007ffff7398e94 in std::_Rb_tree<unsigned long, std::pair<unsigned long const, usrp_ptr>, std::_Select1st<std::pair<unsigned long const, usrp_ptr> >, std::less<unsigned long>, std::allocator<std::pair<unsigned long const, usrp_ptr> > >::_M_erase(std::_Rb_tree_node<std::pair<unsigned long const, usrp_ptr> >*) [clone .isra.0] () from /usr/local/lib/libuhd.so.4.10.0
#20 0x00007ffff739a1a2 in uhd_usrp_free () from /usr/local/lib/libuhd.so.4.10.0
#21 0x000055555576eeb7 in core::ptr::drop_glue::<ezsdr_radio_uhd::uhd::UhdDevice> ()
#22 0x00005555557a12ba in <alloc::sync::Arc<dyn ezsdr_radio_uhd::device::Device>>::drop_slow ()
#23 0x000055555569746b in core::ptr::drop_glue::<ezsdr_radio_uhd::authority::DeviceTime> ()
#24 0x00005555559107fa in <alloc::sync::Arc<dyn ezsdr_kernel::stream::link::DataLink>>::drop_slow ()
#25 0x00005555559047b3 in core::ptr::drop_glue::<ezsdr_kernel::coordinator::state::Shared> ()
#26 0x0000555555910ca0 in <alloc::sync::Arc<ezsdr_kernel::coordinator::state::Shared>>::drop_slow ()
#27 0x000055555596029c in core::ptr::drop_glue::<ezsdr_kernel::coordinator::RunHandle> ()
#28 0x000055555597b95a in <ezsdr_kernel::coordinator::RunHandle>::finish ()
#29 0x000055555567e323 in <hardware::hw_b9_unplug::{closure#0} as core::ops::function::FnOnce<()>>::call_once ()
#30 0x00005555557099fb in test::__rust_begin_short_backtrace::<core::result::Result<(), alloc::string::String>, fn() -> core::result::Result<(), alloc::string::String>> ()
#31 0x0000555555717045 in test::run_test::{closure#0} ()
#32 0x00005555557109f4 in std::sys::backtrace::__rust_begin_short_backtrace::<test::run_test::{closure#1}, ()> ()
#33 0x000055555571a192 in <std::thread::lifecycle::spawn_unchecked<test::run_test::{closure#1}, ()>::{closure#1} as core::ops::function::FnOnce<()>>::call_once::{shim:vtable#0} ()
#34 0x00005555559f093f in <std::sys::thread::unix::Thread>::new::thread_start ()
#35 0x00007ffff69270da in start_thread (arg=<optimized out>) at ./nptl/pthread_create.c:454
#36 0x00007ffff69ba7ac in __GI___clone3 () at ../sysdeps/unix/sysv/linux/x86_64/clone3.S:78
```

- **The cause, VERIFIED (the stack and UHD 4.10's source):** `RunHandle::finish` drops the Run; the last `Arc<dyn Device>`, held through `DeviceTime`, drops `UhdDevice`, which calls `uhd_usrp_free`; that erases the device from the C API's static map, destroying `multi_usrp_rfnoc` and then `rfnoc_graph_impl`, whose destructor shuts every block down (`rfnoc_graph.cpp:122–129`, `block_container.cpp:81–87`, `noc_block_base.cpp:351–357`); the X300 radio's `deinit()` writes its registers (`x300_radio_control.cpp:1890–1911`) with no `try`; over the dead link `ctrlport_endpoint_impl::poke32` throws `uhd::io_error`, which leaves a destructor and so terminates the process.
- So the Run did stop: `finish` had sealed its Manifest (cleanup builds it, `coordinator/mod.rs:319–323`), and the process died while `finish` dropped the Run's state, before returning it. The server writes `manifest.json` only after `finish` returns (`ezsdr-server/src/lib.rs:189–190`, `:227–229`), so in a Session the Manifest and the client's reply would both be lost.


### After F4's fix (`0c2ae4d`) — the receive Run again, under gdb: **`DEVICE_LOST`, the Run stopped by Policy, its Manifest returned**

The owner: "この推奨でOKです．直してください．その後の実機の手順も了解です". A first try ran its 300 s with the cable in (the owner had not seen my message: "ごめんみてなかった"); its log is `~/ezsdr-bench/s2-b9/hw_b9_unplug.after-f4.gdb.log` (`still running after 300 s`, `Stopped { cause: Client }`, no `DEVICE_LOST`). The second, `hw_b9_unplug.after-f4.run2.gdb.log`: started at UTC 1790835361.774; the carrier dropped at 1790835381.853 (pulled at the X300); the test's lines, as printed (UHD's control-timeout errors between them cut, as in the first log):

```
B9: unplug the cable now (receive Run; up to 300 s); started at UTC 1790835361.774
[ERROR] [UHD] An unexpected exception was caught in a task loop.The task loop will now exit, things may not work.EnvironmentError: IOError: 192.168.40.36: x300 fw communication failure #3
B9 receive: not running at UTC 1790835385.864 (Err(Ended { termination: Stopped { cause: Policy { kind: EventKind("DEVICE_LOST") } } }), CleanedUp { termination: Stopped { cause: Policy { kind: EventKind("DEVICE_LOST") } } }); termination Stopped { cause: Policy { kind: EventKind("DEVICE_LOST") } }
B9 receive DEVICE_LOST: [Event { source: ResourceId { node: NodeId(0), path: "usrp" }, time: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 4235276494 }, severity: Fatal, kind: EventKind("DEVICE_LOST"), payload: Object {"message": String("UR-29: the receive stream yielded nothing for 1 s")} }]
B9 receive rejected: [{"reason":"did not join within 1 s; left detached","thread":"uhd-control"},{"because":["uhd-control"],"leaked":"streamers"}]
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 31 filtered out; finished in 27.28s
terminate called after throwing an instance of 'uhd::io_error'
```

- **B9's receive case passes its first two expectations:** `DEVICE_LOST` from `usrp` (UR-29's silence rule: "the receive stream yielded nothing for 1 s"), the Run `Stopped { policy { DEVICE_LOST } }`, and `finish` returned its Manifest — where before the fix the process died inside `finish`.
- **But not within about 1 s:** the Run was found not running at UTC …385.864, **4.0 s after the carrier dropped** (the Run is advanced a second at a time, so the loss was raised 3–4 s after the unplug). UHD's control requests block for its timeouts meanwhile ("x300 fw communication failure #1…#3"); `uhd-control` was inside one and did not join within UR-16's 1 s (`rejected`: `"did not join within 1 s; left detached"`, its streamers leaked). Recorded, not changed (spec 18's numbers are the owner's).
- **Then the process aborted at exit, on the main thread** — the residual UR-29 records: UHD's static map of open devices, destroyed by `exit`, tears down the kept device by the same path. The main thread's stack, as gdb printed it:

```
Thread 1 (Thread 0x7ffff5c6e8c0 (LWP 30) "hardware-4fa4ba"):
#0  __pthread_kill_implementation (threadid=<optimized out>, signo=6, no_tid=0) at ./nptl/pthread_kill.c:44
#1  __pthread_kill_internal (threadid=<optimized out>, signo=6) at ./nptl/pthread_kill.c:89
#2  __GI___pthread_kill (threadid=<optimized out>, signo=signo@entry=6) at ./nptl/pthread_kill.c:100
#3  0x00007ffff68c8b7e in __GI_raise (sig=sig@entry=6) at ../sysdeps/posix/raise.c:26
#4  0x00007ffff68ab8ec in __GI_abort () at ./stdlib/abort.c:77
#5  0x00007ffff5db3275 in ?? () from /usr/lib/x86_64-linux-gnu/libstdc++.so.6
#6  0x00007ffff5dca64a in ?? () from /usr/lib/x86_64-linux-gnu/libstdc++.so.6
#7  0x00007ffff5db2af4 in __cxa_call_terminate () from /usr/lib/x86_64-linux-gnu/libstdc++.so.6
#8  0x00007ffff5dc9b2c in __gxx_personality_v0 () from /usr/lib/x86_64-linux-gnu/libstdc++.so.6
#9  0x00007ffff6bec592 in ?? () from /usr/lib/x86_64-linux-gnu/libgcc_s.so.1
#10 0x00007ffff6bed0db in _Unwind_Resume () from /usr/lib/x86_64-linux-gnu/libgcc_s.so.1
#11 0x00007ffff71e5a72 in ctrlport_endpoint_impl::poke32(unsigned int, unsigned int, uhd::time_spec_t, bool) () from /usr/local/lib/libuhd.so.4.10.0
#12 0x00007ffff798dc11 in x300_radio_control_impl::deinit() () from /usr/local/lib/libuhd.so.4.10.0
#13 0x00007ffff71c8797 in uhd::rfnoc::noc_block_base::shutdown() () from /usr/local/lib/libuhd.so.4.10.0
#14 0x00007ffff7178488 in uhd::rfnoc::detail::block_container_t::shutdown() () from /usr/local/lib/libuhd.so.4.10.0
#15 0x00007ffff71f6824 in rfnoc_graph_impl::~rfnoc_graph_impl() () from /usr/local/lib/libuhd.so.4.10.0
#16 0x00007ffff70fa8d7 in std::_Sp_counted_base<(__gnu_cxx::_Lock_policy)2>::_M_release_last_use_cold() () from /usr/local/lib/libuhd.so.4.10.0
#17 0x00007ffff73737f5 in multi_usrp_rfnoc::~multi_usrp_rfnoc() () from /usr/local/lib/libuhd.so.4.10.0
#18 0x00007ffff70fa8d7 in std::_Sp_counted_base<(__gnu_cxx::_Lock_policy)2>::_M_release_last_use_cold() () from /usr/local/lib/libuhd.so.4.10.0
#19 0x00007ffff73a9d63 in std::map<unsigned long, usrp_ptr, std::less<unsigned long>, std::allocator<std::pair<unsigned long const, usrp_ptr> > >::~map() () from /usr/local/lib/libuhd.so.4.10.0
#20 0x00007ffff68cb5e1 in __run_exit_handlers (status=0, listp=0x7ffff6a95680 <__exit_funcs>, run_list_atexit=run_list_atexit@entry=true, run_dtors=run_dtors@entry=true) at ./stdlib/exit.c:118
#21 0x00007ffff68cb6be in __GI_exit (status=<optimized out>) at ./stdlib/exit.c:148
#22 0x00007ffff68ad608 in __libc_start_call_main (main=main@entry=0x5555556953a0 <main>, argc=argc@entry=6, argv=argv@entry=0x7fffffffe648) at ../sysdeps/nptl/libc_start_call_main.h:83
#23 0x00007ffff68ad718 in __libc_start_main_impl (main=0x5555556953a0 <main>, argc=6, argv=0x7fffffffe648, init=<optimized out>, fini=<optimized out>, rtld_fini=<optimized out>, stack_end=0x7fffffffe638) at ../csu/libc-start.c:360
#24 0x000055555566aef5 in _start ()
```


### Transmit-only Session — `hw_b9_unplug_transmit_only`, under gdb: **`DEVICE_LOST`, the Session stopped by Policy**

The owner: "準備OK", then pulled the cable at the X300. Started at UTC 1790835523.996 (the transmitter enabled, `radio.rx.channels` 0, nothing sent); the carrier dropped at 1790835547.174. The test's lines, as printed (the whole log: `~/ezsdr-bench/s2-b9/hw_b9_unplug_transmit_only.gdb.log`; the stray `D` before the first `B9` is in the log as printed):

```
DB9 transmit only: radio.rx.channels 0: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }
B9: unplug the cable now (transmit-only Session; up to 300 s); started at UTC 1790835523.996
B9 transmit only: not running at UTC 1790835553.193 (Err(Ended { termination: Stopped { cause: Policy { kind: EventKind("DEVICE_LOST") } } }), CleanedUp { termination: Stopped { cause: Policy { kind: EventKind("DEVICE_LOST") } } }); termination Stopped { cause: Policy { kind: EventKind("DEVICE_LOST") } }
B9 transmit only DEVICE_LOST: [Event { source: ResourceId { node: NodeId(0), path: "usrp" }, time: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, ticks: 6239997161 }, severity: Fatal, kind: EventKind("DEVICE_LOST"), payload: Object {"message": String("uhd_usrp_get_time_now: UHD error 30: EnvironmentError: IOError: send error on socket: Network is unreachable")} }]
B9 transmit only rejected: []
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 31 filtered out; finished in 37.08s
terminate called after throwing an instance of 'uhd::io_error'
```

- `DEVICE_LOST` from `usrp`, found by UR-29's device time read (`uhd_usrp_get_time_now: UHD error 30: … Network is unreachable` — `UHD_ERROR_IO`, which UR-29 counts as lost); the Session `Stopped { policy { DEVICE_LOST } }`; `finish` returned; nothing in `rejected`.
- Found not running at UTC …553.193, **6.0 s after the carrier dropped**, against bench.md's "about 1 s": the 500 ms read itself waits out UHD's control timeouts before it returns its error (INFERRED). Recorded, not changed.
- The process then aborted at exit, as in the receive case — UHD's static map, from `exit` (the main thread's frames 11–22):

```
#11 0x00007ffff69ba7ac in __GI___clone3 () at ../sysdeps/unix/sysv/linux/x86_64/clone3.S:78
#11 0x00007ffff71e5a72 in ctrlport_endpoint_impl::poke32(unsigned int, unsigned int, uhd::time_spec_t, bool) () from /usr/local/lib/libuhd.so.4.10.0
#12 0x00007ffff798dc11 in x300_radio_control_impl::deinit() () from /usr/local/lib/libuhd.so.4.10.0
#13 0x00007ffff71c8797 in uhd::rfnoc::noc_block_base::shutdown() () from /usr/local/lib/libuhd.so.4.10.0
#14 0x00007ffff7178488 in uhd::rfnoc::detail::block_container_t::shutdown() () from /usr/local/lib/libuhd.so.4.10.0
#15 0x00007ffff71f6824 in rfnoc_graph_impl::~rfnoc_graph_impl() () from /usr/local/lib/libuhd.so.4.10.0
#16 0x00007ffff70fa8d7 in std::_Sp_counted_base<(__gnu_cxx::_Lock_policy)2>::_M_release_last_use_cold() () from /usr/local/lib/libuhd.so.4.10.0
#17 0x00007ffff73737f5 in multi_usrp_rfnoc::~multi_usrp_rfnoc() () from /usr/local/lib/libuhd.so.4.10.0
#18 0x00007ffff70fa8d7 in std::_Sp_counted_base<(__gnu_cxx::_Lock_policy)2>::_M_release_last_use_cold() () from /usr/local/lib/libuhd.so.4.10.0
#19 0x00007ffff73a9d63 in std::map<unsigned long, usrp_ptr, std::less<unsigned long>, std::allocator<std::pair<unsigned long const, usrp_ptr> > >::~map() () from /usr/local/lib/libuhd.so.4.10.0
#20 0x00007ffff68cb5e1 in __run_exit_handlers (status=0, listp=0x7ffff6a95680 <__exit_funcs>, run_list_atexit=run_list_atexit@entry=true, run_dtors=run_dtors@entry=true) at ./stdlib/exit.c:118
#21 0x00007ffff68cb6be in __GI_exit (status=<optimized out>) at ./stdlib/exit.c:148
#22 0x00007ffff68ad608 in __libc_start_call_main (main=main@entry=0x5555556953a0 <main>, argc=argc@entry=6, argv=argv@entry=0x7fffffffe638) at ../sysdeps/nptl/libc_start_call_main.h:83
```


### The cable back; B3–B7 again: pass, no false `DEVICE_LOST`

The owner: "挿し直しました"; the carrier came back at UTC 1790835576.815, 29.6 s after it dropped. Then B3–B7 (logs in `~/ezsdr-bench/s2-b9/rerun/`):

```
hw_b3_receive_at_t0              test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 31 filtered out; finished in 5.54s
hw_b4_capture_at_a_sample_index  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 31 filtered out; finished in 32.57s   (the reference PLL did not lock: UR-7)
hw_b4_capture_at_a_sample_index  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 31 filtered out; finished in 4.53s   (rerun)
hw_b5_overflow                   test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 31 filtered out; finished in 15.11s
hw_b6_txrx_and_repeat            test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 31 filtered out; finished in 8.54s
hw_b7_session_loopback           test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 31 filtered out; finished in 5.54s
```

No log of the five mentions `DEVICE_LOST`.

### B9 in sum

| Expectation (bench.md B9) | Outcome |
|---|---|
| `DEVICE_LOST` within about 1 s, receive Run | **3–4 s** after the carrier dropped (UR-29's 1 s silence, behind UHD's control timeouts); recorded, not changed |
| `DEVICE_LOST` within about 1 s, transmit-only Session | **6.0 s** (the 500 ms time read waits out UHD's timeouts, then `UHD_ERROR_IO`); recorded, not changed |
| each ending `Stopped { policy { DEVICE_LOST } }` with its Manifest written | yes, both, **after F4's fix** (`0c2ae4d`); before it the process aborted inside `finish` (design-notes §17) |
| no false `DEVICE_LOST` in the rerun of B3–B7 | none |
| the USRP2 probe | not run: no USRP2 on this bench; the owner: "USRP2は必要？X300だけじゃだめ？" — not needed for Gate X; its `pp_string` and profile wait for a USRP2 |
| the UHD error each unplug produced | receive: control timeouts (`x300 fw communication failure #1…#3`), UHD's task loop exiting, then UR-29's silence rule; transmit-only: `uhd_usrp_get_time_now: UHD error 30` (`UHD_ERROR_IO`) |

And what remains, recorded in UR-29: the process aborts at exit in UHD's static teardown while the link is down — after the Manifest, in both cases.

### A correction (Review S, NB-S1)

The latencies above mix the event with the Run's end. From the events' ticks (the time was set before each test's start line): the receive Run's `DEVICE_LOST` (tick 4 235 276 494, 21.176 s after the time was set) came at most **1.10 s after the carrier dropped**, meeting bench.md's "about 1 s"; the "3–4 s" was when the test saw the Run stopped — its 1 s poll, and `Provider::stop` waiting UR-16's 1 s for a uhd-control blocked inside UHD. The transmit-only Session's (tick 6 239 997 161) came **5.8–6.0 s** after it: the event's own latency, the OS's route timeout reaching UHD as `ENETUNREACH`, which Review S's S-B1 explains. The owner's acceptance is withdrawn for the transmit-only figure, to be measured again after S-B1's fix (design-notes §19).

