# USRP2, N2x0, X3x0, X410, X420, X440 against the v4 radio design

Sonnet researcher, 2026-10-16, read-only, at the owner's request ("現状のv4の案でUSRPのデバイスの自由度や能力に制限を加えないか徹底的に調査"). Checked against v4 at `249d0bb` and the P1 (c) direction of `00-overview.md`. VERIFIED here means the researcher read it; the orchestrator's spot checks are in `00-overview.md`'s decision log.

RESULT: v4 today is an X300/X310-only design, and the Phase 8 (c) direction as written keeps that restriction. (c) removes the one-setting-per-direction and one-stream-per-direction limits. It does not touch the master clock, the rate grid, the gain model, the timed-update class, the selector's source names or the throughput model. Each of those is an X3xx constant today.

## Method and caveats

- Primary source is UHD tag v4.10.0.0. `WebFetch` strips line gutters, so I have no `#Lx-Ly` anchors. Each UHD fact cites a file path under `https://github.com/EttusResearch/uhd/blob/v4.10.0.0/`, and doc anchors are named where I have them.
- V = VERIFIED, I read it in the source or doc. I = INFERRED.
- Source key:
  - X3DOX = `host/docs/devices/usrp_x3x0.dox`
  - X4DOX = `host/docs/devices/usrp_x4xx.dox`
  - U2DOX = `host/docs/devices/usrp2.dox`
  - DBDOX = `host/docs/devices/dboards.dox`
  - TWDOX = `host/docs/devices/twinrx.dox`
  - ZBXDOX, HBXDOX, FBXDOX = `host/docs/devices/{zbx,hbx,fbx}.dox`
  - MU = `host/include/uhd/usrp/multi_usrp.hpp`
- The existing note `/home/komatsu/local_work/Ez-SDRv4/plan/phase8/research/x300-uhd-facts.md` agrees with what I re-read: one timekeeper, one DUC per slot, per-port DDC rates, and the dboard clock shared across slots. I reuse its findings and tag them "(note)".

### X420 exists

- UHD 4.10.0.0 lists "Support for USRP X420" as a CHANGELOG highlight (V, `CHANGELOG`, 4.10.0.0 section).
- X4DOX has an X420 section with images and subdev specs, and `host/docs/devices/hbx.dox` documents its daughterboard, the HBX.
- Three FPGA image files exist: `fpga/usrp3/top/x400/x420_{X4_200,X4_1000,CG_1000}_rfnoc_image_core.yml`.
- The Ettus product pages and a NI spec page exist (WebSearch "Ettus USRP X420 X410 X440 UHD 4.10"). Minimum UHD version is inconsistent between sources: 4.9.1 on the product page, 4.10.0 on the NI spec page.
- X420 has two channels, 10 MHz–20 GHz and up to 1 GHz instantaneous bandwidth (V, X4DOX table; HBXDOX).

## 0. v4 today and (c), in brief

What v4 can open today:

- **Profiles and daughterboards.** `x310-ubx` (two UBX), `x310-obx` (one OBX) and `x310-cbx` (one CBX). `crates/ezsdr-radio-uhd/src/profile.rs:25-33`.
- **UR-5 refusals.** Any device whose master clock is not 200 MHz (`provider/mod.rs:139-145`), and any device whose front-end names do not start with `UBX`, `OBX` or `CBX` (`provider/mod.rs:150-168`).
- **Tune.** UHD's AUTO tune policy only: no LO offset, no tune args, no bandwidth, no LO control, no DC or IQ control, no power reference (`uhd.rs:688-695`).
- **Gain and antenna.** One aggregate gain (empty gain name, `uhd.rs:704`). Antenna is set only if the Spec gives one.
- **Motherboard index.** Hard-coded to 0 in `set_command_time`, `set_time_*` and `get_master_clock_rate` (`uhd.rs:120, 411, 622-642, 678`).
- **Clock and time sources.** Only `internal`, `external` and `gpsdo` (`provider/mod.rs:123`).
- **Stream shape.** One streamer per direction over channels `0..n`, all at one rate. Stream args are `fc32` host and `sc16` wire (`uhd.rs:742-744`).

Under (c) (`plan/phase8/00-overview.md` §4 P1):

- **What (c) adds.** Channel nodes (one per front end, with per-channel frequency, gain and antenna and board-type capabilities) and stream nodes (a set of same-rate channels, SampleClock per stream). One profile name per motherboard family (`x3xx`), with the slot and front-end layout in the selector. Link configuration (HG/XG, number of 10 GbE links) in the description. The board-type table lists only ubx, obx, cbx, basic_rx/tx and lf_rx/tx.
- **What (c) does not mention.** MCR, rate grid, image/DDC, gain structure, timed-tune class, per-link throughput, non-X3xx families.

## 1. Per-device tables

Restriction column: none, minor or major. "(c)" means under the Phase 8 direction.

### 1a. X300 / X310 (UHD "X3x0" driver; both are the same code path)

| Capability | UHD fact | v4 today | (c) | Restriction |
|---|---|---|---|---|
| Master clock | 200 MHz default. 184.32 MHz (with a 10, 11.52, 23.04 or 30.72 MHz reference). 120 MHz. "Arbitrary" with a 10 MHz reference (experimental). Set only by the `master_clock_rate` arg at open; runtime `set_rate` is refused. TwinRX allows only 200 MHz. V: `x300_clock_ctrl.cpp` `init()`, `x300_radio_control.cpp` `set_rate()`, X3DOX, TWDOX | No. UR-5 refuses mcr≠200 MHz; the constant is `MASTER_CLOCK_HZ` (`profile.rs:10`) | Unspecified. (c) shares "the motherboard description", but no MCR field is mentioned | minor. An unlisted 184.32 or 120 MHz run is refused; the need is real but rare |
| Sample-rate grid | Rate is MCR/N with N from the DDC's valid set: N in {1} plus 2^h·c, h 0..NUM_HB, c 1..CIC_MAX (V: `host/lib/rfnoc/ddc_block_control.cpp` ctor and `coerce_decim`). The DDC block defaults are NUM_HB=3, CIC_MAX_DECIM=255 (V: `fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_ddc/rfnoc_block_ddc.v`). The X300 image yml sets no override (V: `x3xx_radio_base.yml`, HG yml), so valid N is I: all 1–255, evens ≤510, multiples of 4 ≤1020, multiples of 8 ≤2040 | The grid is 200e6/N for every integer N in 1…512 (`profile.rs:95`; MR-3; `device.rs:234-243`) | Unspecified | **major (looser than hardware).** The grid claims odd N in 257–511, e.g. 200e6/257, which the DDC cannot make. UHD coerces it, UR-12 sees a different read-back and refuses at prepare. So v4 advertises rates the device refuses (the same parity defect Phase 8 targets). It also omits N 513–2040 (down to ~98 kHz) |
| Rate per channel / stream | DDC ports are independent per port: `decim`, `samp_rate` and frequency are per-port vectors (V, note §3). `multi_usrp` harmonises rates inside one streamer (debug log "Inconsistent RX rates … Harmonizing") (V, `multi_usrp_rfnoc.cpp` `get_rx_stream`); class doc says "All channels share a common RX sample rate" (V, MU) | One rate and one stream per direction (RM-3, RM-4) | Yes: rate per stream node, several streams | none after (c) (today: major) |
| Channel counts | RX up to 4 (2 DDC chains/slot × 2 slots). TX up to 2: `duc0`/`duc1` have `NUM_PORTS: 1` and radio input port 1 is unconnected (V, `x300_HG_rfnoc_image_core.yml`; note §3). Basic/LF have RX frontends "0","1" and one TX frontend (V, `db_basic_and_lf.hpp`, note §3). X3DOX's "2 TX DUC chain" is stale (note) | `max_channels` 2 (UBX) or 1 (OBX, CBX), the same for RX and TX (`DeviceDescription.max_channels`, `device.rs:177`) | Per-direction counts follow from the channel nodes | none after (c); today minor (RX capped at 2, TX/RX tied) |
| Daughterboards | Basic/LF, DBSRX2 (needs 100 MHz dboard clock), TVRX2, WBX, SBX, CBX, UBX, OBX, TwinRX (X3xx only, 200 MHz MCR only). XCVR2450 incompatible (V: DBDOX, TWDOX). The dboard clock is per direction and shared by both slots; UBX/OBX/TwinRX request clocks, so mixes can conflict (V: `x300_clock_ctrl.cpp`; X3DOX; note §7). 4.10 lets X310 daughterboards set the dboard clock (V, CHANGELOG 4.10) | UBX, OBX, CBX only (UR-5, `profile.rs:25-33`) | Adds Basic/LF; no WBX, SBX, TwinRX, TVRX2, DBSRX2 | **major today**, minor after (c) (the description list is additive) |
| Tuning, LO and DSP | Two-stage RF LO + DSP NCO via `tune_request_t` policies. UBX has `use_lo_offset=false`, `mode_n=integer` tune arg and `int_n_step` (V, `db_ubx.cpp`). Basic/LF report 0.0 RF and have no LO offset (V, `db_basic_and_lf.cpp`) | AUTO policy only, empty args (`uhd.rs:688-695`). No key for LO offset, int-N or policies (RM-4 is closed) | Same | minor (additive keys later); Basic/LF centre frequency has no sensible meaning in `radio.rx.frequency_hz` unless (c) defines it |
| Frequency range and step | UHD clips to the front-end range widened by the DSP reach, `min(DSP range, bandwidth/2)` (V, `multi_usrp_utils.hpp`, cited by spec 18 UR-5); DSP step is MCR/2^32 | Static `Step` grid per profile, 1 Hz step | Range per board type | none (OBX 8.4 GHz, UBX 6 GHz are already per profile) |
| LO sharing and per-channel independent LOs | TwinRX: LO1/LO2 per channel, sources `internal`, `external`, `companion`, `disabled`, `reimport`; export bool (V, `db_twinrx.cpp`; TWDOX). UBX/OBX: none (V, `db_ubx.cpp`). Phase-sync on a timed tune only for UBX/OBX (V, `db_ubx.cpp` `sync_phase`) | Nothing expressible | (c) channel nodes allow different frequencies per channel. LO sharing couples frequencies across channels and no rule exists | major for TwinRX only; additive keys later |
| Gain | UBX/OBX/CBX/SBX PGA0 0–31.5 dB step 0.5 (V, `db_ubx.cpp`, DBDOX). WBX RX 0–31.5, TX 0–25. TwinRX 0–93 step 1 with profiles low-noise/low-distortion/default (V, `db_twinrx.cpp`). Basic/LF: no gain at all (V, `db_basic_and_lf.cpp`). RX also has ADC digital gain 0–6 dB step 0.5 in the aggregate group (V, `x300_adc_ctrl.cpp`, `x300_radio_control.cpp`) | One static `Step` 0–31.5/0.5, identical for RX and TX (`profile.rs:98`) | Per board-type description; still a static range | minor (Basic/LF needs "no gain"; TwinRX needs step 1 and profiles; profiles additive) |
| Power reference | Supported, manual calibration, keyed by dboard serial (V, X3DOX "Power API") | No key | No | minor (additive) |
| DC offset / IQ balance | Per frontend core, enable bool and complex value, RX and TX (V, `x300_radio_control.cpp`) | No control; UHD's default correction runs unseen, which matters for Vision §26 calibration | No | minor |
| Antennas | UBX TX: TX/RX, CAL. RX: TX/RX, RX2, CAL (V, `db_ubx.cpp`). Basic/LF: AB/BA/A/B (A→I, B→Q) (V). TwinRX: RX1/RX2 per channel with shared amplifiers (V, TWDOX). "Full-duplex forces RX2" (V, DBDOX) | `RX2`, `TX/RX` for RX and `TX/RX` for TX (`profile.rs:100-101`); no CAL | Per-board name lists | minor. The cross-direction coupling on the TX/RX port has no rule today or in (c) |
| Reference and time | Clock sources `internal`, `external`, `gpsdo`; sync pairs internal/internal, external/internal, external/external, gpsdo/gpsdo, gpsdo/internal; external reference 10, 11.52, 23.04, 30.72 MHz (needs `system_ref_rate`); 10 MHz REFOUT; PPS out; internal GPSDO option. Source changes at runtime reset the LMK and ADCs. One timekeeper (V, `x300_mb_controller.cpp`, `bus_int.v` via note §6, X3DOX) | `clock_source`/`time_source` ∈ {internal, external, gpsdo} (UR-5). Reference rate other than 10 MHz and PPS/REFOUT output are not selectable | Unchanged | minor |
| Timed commands | FPGA supports timed commands. Queue per RFNoC block: `radio.yml` `fifo_depth: 512` words (V, note §2). X3DOX warns that repeated timed commands without streaming back up the DDC/DUC queue. `set_command_time` is per mboard and applies to every radio and DDC/DUC (V, `multi_usrp_rfnoc.cpp`) | One device-wide `command_queue_depth` 16 (`profile.rs:116`); Phase 7 measured about 7 timed OBX tunes | Unchanged (device scope) | minor |
| Over-the-wire format | RFNoC DDC `type_in`/`type_out` are constant SC16 (V, `ddc_block_control.cpp`). `rfnoc_graph.cpp` and `rfnoc_rx_streamer.cpp` contain no `sc8`/`sc12` strings. So sc8/sc12 on X3xx under UHD 4 is I: not available | `sc16`, 4 B/sample, fixed (UR-3) | Same | none (sc16 is all UHD 4 offers here) |
| Transport and throughput | HG: port 0 1 GbE, port 1 10 GbE. XG: both 10 GbE. HA/XA: Aurora (built from source). Dual 10 GbE through `second_addr`. PCIe (MXI) and DPDK also work. Documented example is 200 Msps per channel over dual 10 GbE. Link choice is automatic and cannot be set via `multi_usrp`: fewest allocated streams wins, and one streamer can span links (V, X3DOX; note §1) | `rx_bytes_per_s`/`tx_bytes_per_s` 1e9 B/s per direction: one 10 GbE link (`profile.rs:123-127`) | Link configuration goes into the description | major if (c) keeps one aggregate bytes/s per direction. Which channel rides which link is not selectable, so a per-link check is not decidable. One 200 Msps sc16 channel is 800 MB/s against ~1.25 GB/s of line rate |
| RFNoC blocks | HG image has `replay0` (2 ports, DRAM), DDC and DUC (V, `x300_HG_rfnoc_image_core.yml`) | None (spec 18 Not in scope; UR-22 streams repeats from host memory) | 8c | none (deliberately deferred) |
| GPIO and sensors | GPIO bank `FP0`, 12 pins, ATR; dboard banks `RX`, `TX`. Sensors `ref_locked`, `temp_fpga`, dboard `lo_locked` (UBX can intermittently fail to report) (V, X3DOX, `gpio_api.dox`, `x300_mb_controller.cpp`, DBDOX) | Only `ref_locked` is read (UR-27). GPIO deferred (spec 18 §10) | Same | minor |
| Status | Supported, actively maintained. 4.10 adds the dboard-clock setter (V, CHANGELOG) | — | — | — |

### 1b. USRP2 and N200 / N210 (legacy `usrp2` driver)

| Capability | UHD fact | v4 today | (c) | Restriction |
|---|---|---|---|---|
| Master clock | Fixed 100 MHz (V, U2DOX; `host/lib/usrp/usrp2/clock_ctrl.cpp` `get_master_clock_rate()`) | No: UR-5 requires 200 MHz | No | **major (lock-out)** |
| Rate grid | `rx_dsp_core_200`: decim 512 down to 260 step 4, 256 down to 130 step 2, 128 down to ceil(tick/link) step 1. Max N 512, so ≥ 195.3 kHz. Halfbands when decim is even (V). Rate = 100e6/N | The grid is 200e6/N, N 1–512; all values differ | No | major |
| Channels | 1 slot "A". 2 RX DDC chains, 1 TX DUC (V, U2DOX, `usrp2_impl.cpp`). RX 2 channels needs a two-frontend board (U2DOX example TVRX2) or Basic/LF subdev modes | `max_channels` 2 per direction | Per-direction channel counts could express 2/1 | minor |
| Daughterboards | U2DOX names Basic/LF (mode via subdev spec), TVRX2 and the legacy boards in DBDOX (DBSRX, DBSRX2, RFX, XCVR2450, WBX, SBX). UBX/CBX/OBX compatibility on N2xx is I; the NI page lists WBX, SBX, UBX, TVRX2, DBSRX2, XCVR2450, LFRX/LFTX. My WebSearch found no authoritative matrix (KB pages returned 403) | UBX/OBX/CBX behind UR-5's 200 MHz refusal | Not in the board-type table | major |
| Tuning, gain, antenna | N2xx has a DAC with interpolation and TX NCO zones (V, `usrp2_impl.cpp` `set_tx_dsp_freq`); RX ADC digital gain 0–6 dB and fine gain on N200/N210 (V). RX frequency range is ±(tick/2) on the DSP (V, `rx_dsp_core_200.cpp`) | Same single-scalar model | Same | minor |
| Reference and time | Clock sources `internal`, `external`, `mimo`, `gpsdo` (N2xx only); time source default `none`, `external`, `mimo`, `gpsdo`. Sensors `ref_locked`, `mimo_locked` (V, `usrp2_impl.cpp`, U2DOX). MIMO cable shares clock, time and optionally Ethernet; the slave uses `set_time_source("mimo",1)` | Selector accepts only `internal`, `external`, `gpsdo`. `internal` as a time source is not a name the driver lists (INFERRED from "defaults to none"), so the default would fail | No | **major** (no `mimo`, wrong time-source default) |
| Timed commands | `time/cmd` is registered only for N200, N210 and their R4 revisions; USRP2 REV3/4 have none (V, `usrp2_impl.cpp`). Timed streaming and sampling exist | RM-4 declares `hardware_timed` for frequency and gain on every device | No per-device class | major for USRP2 (a timed tune cannot be honoured), none for N2xx |
| Wire format | `rx_dsp_core_200`: sc16 (word 0) and sc8 (word 1); sc12 throws (V). Bandwidth 25 MHz at sc16, 50 MHz at sc8 (V, U2DOX). 1 GbE only: USRP2 refuses 10/100 (V) | `sc16`, 4 B/sample; the sc8 selector is deferred (spec 18 §10) | No | **major**: 1 GbE at sc16 caps near 25 Msps; sc8 doubles it and cannot be chosen |
| Transport | 1 GbE only (a shared Ethernet mode over the MIMO cable). Default IP 192.168.10.2 | `rx_bytes_per_s` 1e9 | No | major (needs ~1.25e8 B/s) |
| GPIO and sensors | U2DOX does not mention GPIO (searched: the page text). `gpio_api.dox` also does not list N2xx | Deferred | — | n/a |
| Status | Source and docs present in 4.10 (`host/lib/usrp/usrp2/` with 18 files). The 4.10.0.0 CHANGELOG section has no USRP2/N200/N210/deprecation/removal entry (searched the first 100 000 of 163 589 characters, which includes the 4.10 section) | — | — | — |

N200 vs N210: UHD treats them identically (same codec, same `time/cmd`). The FPGA-size difference is I and not read from UHD.

### 1c. X410

| Capability | UHD fact | v4 today | (c) | Restriction |
|---|---|---|---|---|
| Master clock | 245.76 or 250 MHz on 200 MHz images; 491.52 or 500 MHz on 400 MHz images (V, X4DOX `x410_usage_mcrs`). Invalid MCR fails the session; no runtime change. Up to 500 Msps, ~400 MHz bandwidth | No: UR-5 refuses mcr≠200 MHz. Also spec 18's UR-3 proof of an exact time-spec round trip assumes `mcr < 2^28` (268 435 456); 491.52e6 and 500e6 exceed it (I: the proof bound, not necessarily behaviour) | Unspecified | **major (lock-out)** |
| Rate grid | Radio rate equals MCR (V, `x400_radio_control.cpp`). Lower rates only through DDC/DUC, present on `X4_200` and `UC_200` images, absent on `CG_400` (V, X4DOX image table) | One grid, 200e6/N | No | major |
| Channels | 2 ZBX × 2 = 4 RX + 4 TX; subdev specs `A:0 A:1 B:0 B:1` (V, X4DOX) | `max_channels` 2 | Channel nodes would express 4 | minor after (c) |
| Daughterboard | ZBX only: 1 MHz–7.2 GHz (LOs to 8 GHz), 400 MHz bandwidth, 2 independent chains per board (V, ZBXDOX) | Not in profile list | Not in the (c) table | major until added |
| Tuning and LO | Superheterodyne, LMX2572 LOs; the LO setting is quantised to the reference. Per-channel LOs are deliberately offset to avoid injection locking. `set_*_lo_source` takes `internal`/`external` (V, ZBXDOX; `zbx_dboard_init.cpp`). The RFSoC NCO does the final shift. **Timed tuning is not supported** (V, ZBXDOX, X4DOX) | RM-4 sets `radio.*.frequency_hz` to `hardware_timed` for every Provider. Spec 18 UR-24 queues a timed tune; the device cannot honour it | Unchanged | **major**: a timed tune is accepted at validate and cannot be executed; the class is per key, not per device |
| Gain | Overall 0–60 dB. TX 2×31 dB DSAs plus amp; RX 4×15 dB DSAs, RX limited to 0–38 below 500 MHz. Profiles `default`, `manual`, `table`, `table_noatr` (V, ZBXDOX) | One static 0–31.5/0.5 range | Static per board type | major (range depends on frequency; profiles are unreachable) |
| Antennas | `TX/RX0`, `RX1`, `CAL_LOOPBACK`, `TERMINATION` (V, ZBXDOX) | `RX2`, `TX/RX` | Per-board lists | minor |
| Power reference, DC, IQ | Power API supported with nominal calibration; DC/IQ not documented in ZBXDOX | None | None | minor |
| Reference and time | Time sources `internal`, `external`, `qsfp0`, `gpsdo`; clock sources `mboard`, `internal`, `external`, `nsync` (not functional), `gpsdo`. One timekeeper on X410. Changing clock/time source triggers ADC self-calibration (since 4.5) (V, X4DOX) | Selector accepts only internal/external/gpsdo; the `startup_latency_ns` 2 s (UR-9) does not account for self-calibration | No | major (`qsfp0`, `mboard`), minor (calibration time is untested) |
| Over-the-wire | No sc12 mention in X4DOX; RFNoC formats as for X3xx (I: sc16 only) | sc16 | Same | none |
| Transport | QSFP28 ×2, 4 lanes each. `X4_200`: 4×10 GbE on port 0. `UC_200`: 100 GbE on port 1. `CG_400`: 100 GbE on both. RJ45 ~10 Msps. DPDK supported (V, X4DOX) | One aggregate `bytes/s` | Link configuration in the description | **major**: 4×10 GbE means each channel is a flow on one lane. One aggregate check cannot say a single 500 Msps channel (2 GB/s) exceeds one 10 GbE link (I) |
| RFNoC | Replay: 4 GiB 4-channel on `X4_200`/`UC_200`; none on `CG_400`; DDC/DUC as above (V, X4DOX image table; `x410_X4_200_rfnoc_image_core.yml`) | None | 8c | none (deferred) |
| GPIO and sensors | Two HDMI banks GPIO0/GPIO1, 12 pins each; sources `DBx_RFy`, `DBx_SPI`, `PS`, `USER_APP`; up to 4 SPI peripherals (V, `x400_gpio_api.dox`). Sensors `lo1_locked`, `lo2_locked`, `nco_locked`, temperatures (V, ZBXDOX) | Deferred; Vision §38 already says X4x0 GPIO is not hardware-timed (only ATR is) | — | minor |
| Status | Supported since UHD 4.1 (WebSearch: Ettus X410 guide). Needs embedded MPM | — | — | — |

### 1d. X420

| Capability | UHD fact | v4 today | (c) | Restriction |
|---|---|---|---|---|
| Master clock | Default 1250 MHz on `xx_1000` images, 250 MHz on `X4_200`; also 245.76, 250, 491.52; converter rates 1966.08/2000/3932.16/2500 MHz (V, X4DOX `x420_usage_mcrs`). Up to 1250 Msps | No (UR-5, and UR-3's `mcr < 2^28` proof bound) | Unspecified | major (lock-out) |
| Rate grid | Radio rate equals MCR; DDC/DUC only on `X4_200`; `X4_1000` and `CG_1000` have none (V) | One grid | No | major |
| Channels | 2 (one HBX per slot; subdev `A:0`, `B:0`) (V, X4DOX, HBXDOX) | `max_channels` | Channel nodes | none after (c) |
| Daughterboard | HBX: 10 MHz–20 GHz; 1 GHz bandwidth above 2.2 GHz, 200 MHz at or below 2.2 GHz; direct sampling below 500 MHz (V, HBXDOX) | Not in list | Not in table | major until added |
| Tuning and LO | LO/NCO; **timed tuning supported above 500 MHz only** (V, X4DOX `x4xx_usage_timedcmds`). TX and RX LOs are independent; export with `set_tx_lo_export_enabled(true,"HBX_LO",ch)`, import with `set_tx_lo_source("external","HBX_LO",ch)`; LO input/output SMAs; the LO frequency is 1×, ½× or ¼× the centre by band (V, HBXDOX) | `hardware_timed` for frequency would be partly true; a below-500 MHz timed tune fails | No LO keys | major (timed class depends on frequency); minor for LO sharing (additive keys) |
| Gain | RX 0–48 dB, TX 0–68 dB, band-dependent. TX steps are 2 dB below 6 GHz (V, HBXDOX, DBDOX). Profiles `default` and `manual` | Static scalar range 0–31.5/0.5 | Static per type | major (range depends on band) |
| Antennas | `TX/RX0`, `RX1`, `CAL_LOOPBACK`, `TERMINATION`, `SYNC_INT` (V, HBXDOX) | `RX2`, `TX/RX` | Per-board lists | minor |
| Reference and time | **Two timekeepers** (one per daughterboard) that start in lockstep; use `set_time_next_pps`/`set_time_unknown_pps` to keep them aligned (V, X4DOX) | One Root with `get_time_now(mboard 0)` reads only timekeeper 0 (`uhd.rs:638-643`) | Unspecified | minor (I: they stay locked after the PPS set) |
| Transport | `X4_200` 4×10 GbE, `X4_1000` 4×10 GbE, `CG_1000` 100 GbE on both ports. At 1250 Msps × 4 B each channel is 5 GB/s | One aggregate `bytes/s` | Link configuration | major (as for X410) |
| RFNoC | 8 GiB 2-channel Replay on every image (V, X4DOX) | None | 8c | none (deferred) |
| Sensors | `lo_locked`, `nco_locked`, `temperature*`, `lo_pd` (V, HBXDOX) | Only `ref_locked` | — | minor |
| Status | New in 4.10.0.0 (V, CHANGELOG) | — | — | — |

### 1e. X440

| Capability | UHD fact | v4 today | (c) | Restriction |
|---|---|---|---|---|
| Master clock | A fixed set from 125 Msps to 2.048 Gsps (converter 1–4.096 GHz divided by 2, 4 or 8). Requested rates are coerced to the next available MCR; the `converter_rate` arg overrides the default. **Dual-rate:** each daughterboard may have its own MCR (`master_clock_rate=1024e6;1280e6`), and dual-rate disables MTS (V, X4DOX `x440_usage_mcr_general`, `x440_usage_mcr_dualrate`) | No (UR-5; the `mcr < 2^28` proof bound is exceeded) | One motherboard description, so one MCR | **major**: dual rate means two tick rates under one motherboard. UR-7 registers exactly one Root at `master_clock_rate/1` (I: no way to express a second radio clock) |
| Rate grid | Radio rate equals MCR; DDC/DUC only on `X4_200` (V, X4DOX) | One grid | No | major |
| Channels | 8 (`A:0`–`A:3`, `B:0`–`B:3`) on `xx_400` images; 2 (`A:0`, `B:0` only) on `xx_1600` images (V, X4DOX) | `max_channels` | Channel nodes | none after (c) |
| Daughterboard | FBX: 30 MHz–4 GHz (UHD tune range 1 MHz–4 GHz), 4 channels each, no gain stages, no filters, direct IF sampling, passband ~0.4 Fc per Nyquist zone (V, FBXDOX) | Not in list | Not in table | major until added |
| Tuning | RFSoC NCO only; **no timed tuning** (V, FBXDOX, X4DOX) | `hardware_timed` for frequency would be wrong | Unchanged | **major** (as X410) |
| Gain | FBX has none (V, FBXDOX) | Static 0–31.5 | Per board type: can say "no gain" | minor after (c) |
| Antennas | `TX/RX0`, `RX1` per channel, `CAL_LOOPBACK`, `TERMINATION`; one SYNC IN port (V, FBXDOX) | `RX2`, `TX/RX` | Per-board lists | minor |
| Reference and time | Two timekeepers (one per daughterboard); the ADC self-calibration runs at startup and on clocking changes (V, X4DOX) | As for X420 | — | minor |
| Transport | `X4_200`, `X4_400`, `X4_1600`: 4×10 GbE; `CG_400`, `CG_1600`: 100 GbE on both ports (V, X4DOX). At 2.048 Gsps sc16 is 8 GB/s per channel | One aggregate `bytes/s` | Link configuration | major |
| RFNoC | 8 GiB Replay on `X4_400` (8 channels, two 4-channel blocks) and `X4_1600` (2 channels); none on `CG_*` or `X4_200`. With timed complex gain on all 8 channels of `x440_X4_400`, crossbar connections limit to matching indices (V, CHANGELOG 4.10) | None | 8c | none (deferred) |
| Sensors and GPIO | `nco_locked`, virtual `lo_locked`, `temperature`, `rfdc_rate` (the last one reports radio 0 only in dual-rate) (V, FBXDOX, X4DOX). GPIO as X410, SPI divider is MCR/8 (V, `x400_gpio_api.dox`) | Only `ref_locked` | — | minor |
| Status | Supported since 4.5 (WebSearch: Ettus X410/X440 guide); X4DOX table headings "As of UHD 4.6" | — | — | — |

## 2. Cross-cutting: v4 rules and constants that restrict some device

| # | Rule or value (where) | Devices | (c) |
|---|---|---|---|
| X1 | `mcr == 200 MHz` (UR-5; `profile.rs:10`; `provider/mod.rs:139-145`). UR-3's exact-round-trip argument also assumes `mcr < 2^28` | all except X3xx at default MCR | keeps, unless the description gains an MCR |
| X2 | Rate grid `200e6/N`, N 1–512 (MR-3; `profile.rs:95`; UR-12; `device.rs:234-243`) | All. It is wrong for X3xx too: odd N 257–511 are advertised and invalid, N>512 omitted | keeps. (c) puts rates on streams but does not change the grid |
| X3 | One frequency, gain, antenna per direction (RM-4, RM-5) | X3xx mixed boards, TwinRX, ZBX, HBX, FBX | **removes** |
| X4 | One rate and one stream per direction, one streamer per direction (RM-3; UR-2 `rx_open(channels)`) | X3xx per-port DDC rates, X440 dual-rate | removes for single-MCR devices |
| X5 | `max_channels` shared by RX and TX (`device.rs:177`; UR-5 `have ≥ need`) | X3xx (RX 4, TX 2), N2xx (RX 2, TX 1), Basic/LF TX (1) | removes (channel nodes carry counts) |
| X6 | Gain is one static `Step` range (RM-4, RM-8; `profile.rs:98`) | ZBX/HBX (frequency-dependent), TwinRX (step 1, 0–93), Basic/LF, FBX (none), WBX TX 0–25 | keeps (per-board static ranges only) |
| X7 | Closed key table (RM-4 "exactly these"): no bandwidth, LO source/export/offset, tune policy, int-N, DC/IQ, power reference, gain profile/stage names, ADC self-cal control | TwinRX, HBX, ZBX, UBX/OBX, all | keeps (additive minor version) |
| X8 | Update class is per key: `radio.{rx,tx}.frequency_hz` and `gain_db` `hardware_timed` for every Provider (RM-4, RM-21, UC-6). `radio.hardware_time` is not tied to it (grep `hardware_time`: RM-4, RM-26, MR-3 only) | X410, X440 (no timed tune), X420 (timed above 500 MHz only), USRP2 (no `time/cmd`) | keeps |
| X9 | `radio.phase_behavior_on_retune` has two values, no `random` (RM-4/RM-9; `profile.rs:76-78`). Phase-sync on timed tune exists only for UBX/OBX (V, `db_ubx.cpp`) | CBX, X410, X440, USRP2 | keeps (Phase 8 excludes `random`) |
| X10 | Selector sources `internal`/`external`/`gpsdo` only (UR-5; `provider/mod.rs:123`) | N2xx/USRP2 (`mimo`, time source `none`), X4xx (`mboard`, `qsfp0`) | keeps |
| X11 | Motherboard index 0 (`uhd.rs:120, 411, 622-642, 678`); one Root at `mcr/1` (UR-7) | Multi-mboard (8b), X420/X440 two timekeepers, X440 dual-rate | keeps (8b) |
| X12 | `PerformanceEnvelope` is aggregate per direction with a fixed 4 B/sample (RM-7; `profile.rs:123-127`); no sc8/sc12 selector (spec 18 §10) | N2xx (sc8 doubles the rate), X4xx per-lane 10 GbE | partly: link configuration enters the description; the check stays aggregate |
| X13 | Front-end check is a name-prefix match per profile (UR-5) | every non-UBX/OBX/CBX board | replaced by the selector's slot/front-end layout |
| X14 | `TimingEnvelope` is device-wide (one queue depth, one lead, one start lead; the per-direction start lead is queued, not done) | X3xx per-block queues, X4xx per-radio timekeepers | keeps |
| X15 | RM-19 `RfEnvelope` (`max_gain_db` "applies to every transmit channel", `radio.tx.frequency_hz` scalar) | any device after (c) gives per-channel values | **must change in (c)** |
| X16 | Antenna names `RX2`, `TX/RX` hard-coded; no CAL; no cross-direction coupling (`profile.rs:100-101`) | all non-UBX boards | per-board lists, coupling unspecified |
| X17 | `Device` surface lacks: bandwidth, LO, DC/IQ, power reference, profiles, sensors other than `ref_locked`, GPIO, wire-format choice (UR-2) | all | keeps |

## 3. What (c)'s amendment must add

Design blockers (decide now, because retrofitting would change tree shape, ids, or a closed schema):

1. **Master clock and image as description inputs, not constants.** The rate grid is a function of (MCR, image), and the grid itself is X3xx-specific today (X1, X2). Fix the X3xx grid at the same time: N must come from the DDC valid-decimation set, or the Mock stays looser than hardware. Name profiles by family without encoding "200 MHz".
2. **A per-device timed-update rule.** Either RM-4's class is made per device, or a capability tells validate that a key's update cannot be timed (X8). Without it, X410, X440 and USRP2 accept a timed tune and cannot honour it. An additive key is fine only if the matcher refuses before the Run.
3. **Id scheme of channel and stream nodes including the motherboard.** Specs, Session targets and fixtures name nodes. 8b needs a multi-mboard identity (X11), and X3xx channel order follows the subdev spec (V, `multi_usrp_rfnoc.cpp`). A breaking rename after fixtures exist is cheap pre-freeze but costs every Spec.
4. **Shared-resource limits across stream nodes (Sol's finding 7), including per-link.** The aggregate formula RM-7 cannot decide X4xx per-lane flows or X3xx link assignment, which `multi_usrp` does not expose (X12).
5. **RM-19 per channel** (X15). It is inside (c)'s scope and has a closed schema (`schemas/radio/rf_envelope.v1.json`, RM-20).
6. **Cross-channel constraints inside one stream.** TwinRX antenna sharing, the TX/RX port shared by UBX transmit and receive, and ADC digital gain written to both ADC channels (V, `x300_adc_ctrl.cpp`). (c)'s "independent per-channel settings" is false for these; at least the constraint form must be decided.
7. **Timekeeper/root model for X420 and X440 and dual-rate X440.** One Root at `mcr/1` cannot express two radio clocks (X11). Decide whether a motherboard may own several roots.

Missing features that can follow additively (RM-4 allows an additive minor version):

- Gain profile and stage keys, power reference, bandwidth, LO source/export/offset, int-N and DC/IQ keys (X6, X7).
- A `random` value for phase behaviour (X9).
- Board-type rows for TwinRX, WBX, SBX, TVRX2, DBSRX2, ZBX, HBX, FBX.
- Selector values `mimo`, `qsfp0`, `mboard` and the `none` time source (X10).
- A wire-format selector (N2xx sc8) (X12).
- GPIO as a sub-resource of the radio (Vision §39), and RFNoC Replay/DDC/DUC (8c).

## 4. Searches that found nothing, or sources I could not read

- UHD CHANGELOG 4.10.0.0: searched the visible text for "USRP2", "N200", "N210", "deprecat", "removed". No match in the 4.10 section; the last 63 589 characters (older sections) are unread. The Ettus end-of-life status of USRP2 is not verified.
- `usrp2.dox`: searched for GPIO, deprecated and obsolete. Absent. `gpio_api.dox` lists neither "RX/TX banks" nor N2xx. Timed GPIO commands: not covered in `gpio_api.dox` or `x400_gpio_api.dox`. Vision §38's statement that GPIO is `hardware_timed` on X3x0 is therefore unsupported by these docs (not contradicted either).
- sc8/sc12 on RFNoC devices: no match in X3DOX, X4DOX, `rfnoc_graph.cpp`, `rfnoc_rx_streamer.cpp`. `rfnoc_streamer.cpp` and `rx_streamer_impl.hpp` returned 404. The "sc16 only" claim rests on the DDC's constant SC16 types (V) and is otherwise I.
- X4xx DDC `NUM_HB` / `CIC_MAX_DECIM` values: not in `x410_X4_200_rfnoc_image_core.yml`; the X4xx DDC grid is unread.
- LO export on ZBX: not found in `zbx_dboard_init.cpp` or ZBXDOX (LO source `internal`/`external` is there). TwinRX timed tune: TWDOX is silent.
- Daughterboard-to-motherboard compatibility matrix: `kb.ettus.com/Daughterboards_and_Motherboards_Compatibility` and `kb.ettus.com/USRP_N200/N210/N200-R/N210-R` returned HTTP 403. N2xx daughterboard support beyond U2DOX is I.
- `ref_locked` sensor on X4xx and the rest of the N2xx/X4xx tuning details beyond the files named above: not read.

## PREVIOUS_HYPOTHESIS_CHECK

I tested your (c) direction against UHD, not an earlier hypothesis. (c) does fix the X3xx mixed-board case. The one-stream-per-direction restriction also hits X3xx itself, because its DDC ports have independent rates (V). The assumption most likely to fail is "independent per-channel settings" for TwinRX, UBX transmit/receive and the ADC digital gain. The restriction that bites X410/X420/X440 and USRP2 regardless of (c) is X1, X2 and X8.

Sources:
- https://github.com/EttusResearch/uhd/tree/v4.10.0.0 (files named above, all at `blob/v4.10.0.0/`)
- https://www.ni.com/docs/bundle/usrp-x420-specs/page/specs.html
- https://www.ettus.com/?p=11354
- https://kb.ettus.com/USRP_X410_Getting_Started_Guide
- https://lists.ettus.com/empathy/thread/XBW4GRRO2TLMOF46WABR7HDSWCW72DU7
- https://www.mathworks.com/help/comm/usrpradio/ug/supported-hardware-and-required-software.html
- https://kb.ettus.com/N200/N210
- `/home/komatsu/local_work/Ez-SDRv4/plan/phase8/research/x300-uhd-facts.md`
