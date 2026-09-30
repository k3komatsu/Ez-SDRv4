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

## B6, B7 (Rust), B8 — not run yet

The session's script ran them from the wrong directory (`cargo` found no `Cargo.toml` in `/bench`): nothing was transmitted by them. Next: rerun them with the working directory `/work`.

## B7 (Python) — ran; one finding

`EZSDR_SERVER=/cargo-target/release/ezsdr-server EZSDR_PROFILE=/bench/bench-session.json` (bench.md's Session profile with `x310-obx`, `addr=192.168.40.36`, the capture directory `/bench/b7-captures`):

- `minimal.py` (Vision §3's snippet, unchanged): `captured 100000 samples, mean power 0.0001` (the 0.5-amplitude tone through 30 dB), 3 logged calls; Manifest `~/ezsdr-bench/ezsdr-runs/session-7-0/manifest.json`.
- `bench_loopback.py`: the rate 19.5 Msps coerced to 20 Msps (a new SampleClock); **TIME_ERROR events: 0** (spike K6); y's correlation peak at sample 7 801, z's at 6 773; `rec_0` first sample tick 54 543, `rec_1` 179 571 (receive clock); asked y at root tick 411 438 339, z at 436 443 948; Manifest `~/ezsdr-bench/ezsdr-runs/session-7-1/manifest.json`.
- **Finding: "the retune outside the RF envelope was admitted".** Not a Module defect: the example retunes `sdr.rx.frequency`, and RM-19 limits only transmit frequencies ("Receive frequencies are not limited: the envelope is about emission", design/07). `hw_b7_session_loopback` retunes `radio.tx.frequency_hz`, which is the check bench.md asks for. Fix the example (`sdr.tx.frequency = 2.4e9`), then run it again.
- Whether z started at t (EA-17): not yet derived from the numbers above (the two clocks' origins are in the Manifests).

## Still to do

1. Rerun B6, B7 (Rust) and B8 from `/work`.
2. Fix `python/examples/bench_loopback.py` to retune the transmitter; rerun B7 (Python); derive EA-17's answer from the Manifests.
3. B9: the unplug (manual: the owner pulls the 10 GbE cable during a receive Run, and during a transmit-only Session — the latter has no test yet), the rerun of B3–B7 with no false `DEVICE_LOST`; the USRP2 probe if a USRP2 is at hand.
4. Fill the table at the end of `plan/spikes/2026-09-26-uhd.md`; update handoff.md.
