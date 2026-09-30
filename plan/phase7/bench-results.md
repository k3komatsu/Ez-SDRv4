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
- **UR-25 finding: the X300 did not honour the timed stop of the continuous stream.** `timing`: `rx_stop` issued at tick 403 313 197 for 403 513 197; samples past the cut arrived and uhd-rx fell back to `rx_stop_untimed` at 404 292 083 (3.9 ms after the cut), as UR-25 provides. The step passes; whether UHD 4.10's RFNoC radio ignores a stop's time spec or honours it late is for B8's receive-stop measurement.

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

First run (as `4a9f2b2`): `B7 time errors: []`, `test result: ok. 1 passed … finished in 17.56s`. The test prints nothing else, so `println!`s were added (test output only: the refusal, `diagnose` of the capture, the capture's continuity and the Manifest's sample clocks) and it was run twice more; the last run:

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
- **UR-25 again: the timed receive stop is never honoured.** In all 7 Runs of run 1 the stop was issued 1.000 ms before its cut and uhd-rx fell back to `rx_stop_untimed` when samples past the cut arrived, 2.78, 3.35, 2.74, 2.36, 3.61, 2.00, 3.45 ms after the cut (B3's was 3.9 ms). With B3 that is 8 of 8: the X300 on UHD 4.10 ignores (or does not honour within ~2–4 ms) the time spec of a continuous stream's stop. An input for Phase 8 and for the owner (UR-25 says it is INFERRED); the Module's fallback works.
- The transmit SampleClock's origin (`enabled`) is 2.4–4.5 ms after T0 in the 7 Runs.
- **Not implemented, so not measured** (bench.md B8's other rows, handoff.md §4's B8 row): timed retunes at the leads; `cold` restart leads 100/50/25/10 ms; the transmit end after `Stop`; in-flight windows 10/5/3/2 ms against `TX_UNDERFLOW`; the preemption bound at 20/10/5 ms; the queue depth of timed OBX tunes (UR-24); a dedicated timed-receive-stop measurement (answered above from the `timing` of every Run instead). Spec 18 §3's restart lead 50 ms, in-flight window 10 ms, release window 3 ms and queue depth 16 stay INFERRED.

## B7 (Python) — ran; one finding

`EZSDR_SERVER=/cargo-target/release/ezsdr-server EZSDR_PROFILE=/bench/bench-session.json` (bench.md's Session profile with `x310-obx`, `addr=192.168.40.36`, the capture directory `/bench/b7-captures`):

- `minimal.py` (Vision §3's snippet, unchanged): `captured 100000 samples, mean power 0.0001` (the 0.5-amplitude tone through 30 dB), 3 logged calls; Manifest `~/ezsdr-bench/ezsdr-runs/session-7-0/manifest.json`.
- `bench_loopback.py`: the rate 19.5 Msps coerced to 20 Msps (a new SampleClock); **TIME_ERROR events: 0** (spike K6); y's correlation peak at sample 7 801, z's at 6 773; `rec_0` first sample tick 54 543, `rec_1` 179 571 (receive clock); asked y at root tick 411 438 339, z at 436 443 948; Manifest `~/ezsdr-bench/ezsdr-runs/session-7-1/manifest.json`.
- **Finding: "the retune outside the RF envelope was admitted".** Not a Module defect: the example retunes `sdr.rx.frequency`, and RM-19 limits only transmit frequencies ("Receive frequencies are not limited: the envelope is about emission", design/07). `hw_b7_session_loopback` retunes `radio.tx.frequency_hz`, which is the check bench.md asks for. Fix the example (`sdr.tx.frequency = 2.4e9`), then run it again.
- Whether z started at t (EA-17): derived in session 2 from `session-7-1`'s Manifest, below.

### B7 (Python), session 2 rerun with the fixed example (`36174d1`): pass

Same container and environment as session 1 (`-w /bench -e PYTHONDONTWRITEBYTECODE=1 -e PYTHONPATH=/work/python -e EZSDR_SERVER=/cargo-target/release/ezsdr-server -e EZSDR_PROFILE=/bench/bench-session.json`; the release server rebuilt from `36174d1`, the server code unchanged since session 1). `bench_loopback.py` now retunes `sdr.tx.frequency = 2.4e9` (the fix: session 1's finding (c); the v58 scan and the Python suite, 23 tests against the debug server, pass). Session 1's two logs are kept as `~/ezsdr-bench/b7_minimal.s1.log` and `b7_loopback.s1.log`.

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

Every capture is 20 000 samples with no gap. So a capture requested at an instant the Run has just reached (`t` returned by `sleep`) still starts there: the local request does not outrun the device's receive latency on this bench (the answer to EA-17's INFERRED case; the samples at `t` had not yet been delivered to the recorder when the request reached it, or the recorder kept them).

## Still to do

1. ~~Rerun B6, B7 (Rust) and B8 from `/work`.~~ Done in session 2 (B6 after a fix of the test's correlation).
2. ~~Fix `python/examples/bench_loopback.py` to retune the transmitter; rerun B7 (Python); derive EA-17's answer from the Manifests.~~ Done in session 2.
3. B9: the unplug (manual: the owner pulls the 10 GbE cable during a receive Run, and during a transmit-only Session — the latter has no test yet), the rerun of B3–B7 with no false `DEVICE_LOST`; the USRP2 probe if a USRP2 is at hand.
4. Fill the table at the end of `plan/spikes/2026-09-26-uhd.md`; update handoff.md.
