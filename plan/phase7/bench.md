# Phase 7 — the first bench session

The one part of Phase 7 that needs hardware: an X310 (or an X300) with **one OBX** daughterboard in slot A, cabled in loopback, on a Linux PC, driven by the implementation of specs 18 and 19 once §9's steps 1–6 of [`00-overview.md`](00-overview.md) are done. Everything before this page runs on a Mac without a USRP. The session answers two questions: do the acceptance crate's experiments and the Phase 6 snippets run on the X310 with only the profile changed (Vision §59; Gate X's criterion 9 if the owner accepts T18), and what are the values spec 18 marks INFERRED (inputs for Phase 8, not changes to any profile now).

**The bench is one OBX, not two UBX** (the owner, 2026-09-30: first one CBX, "UBX2枚での試験ではなくCBX1枚でループバックで可能なように", then "了解ですOBXに切り替えてください"). Its profile is `x310-obx` 0.1.0 (spec 18 UR-9): `x310-ubx`'s values with one channel each way and 10 MHz–8.4 GHz (UHD 4.10's `obx_freq_range`, `db_obx.hpp`, VERIFIED), the default frequency RM-5's 1 GHz. The OBX is the UBX's design grown: two MAX2871 LOs each way and the same timed LO phase sync (`obx_expert.cpp`'s `_sync_phase`, VERIFIED), so the numbers the session measures are an OBX's but come from the same kind of synthesizer as the UBX's (INFERRED: their tune code differs). UR-5 refuses a profile whose channels or front ends the device does not have. `x310-ubx` stays unmeasured; design-notes §9–§10 record what that leaves for Phase 8. The hardware tests take the profile from the front end UHD names on receive channel 0 (`OBX…` is `x310-obx`, `CBX…` `x310-cbx`, anything else `x310-ubx`) and run at that profile's default frequency; B1 fails unless it is `x310-obx`. **UHD 4.9 or later** drives an OBX (UHD added it in June 2025; an older UHD names it an unknown board, which UR-5 refuses).

**An X300 serves as well** (the owner, 2026-09-30: "USRP X310の代わりにUSRP X300でもいいですか？" — yes). UHD 4.10 drives both with its one `x300` driver, telling them apart only by the EEPROM's product code and the FPGA image (`x300_mboard_type.cpp`, VERIFIED); the X300's FPGA is the smaller XC7K325T, which matters only for RFNoC blocks such as Replay that this Module does not use (the repeat is streamed from host memory, spec 18 UR-22; INFERRED that the X300's default image carries what the steps need). UR-5 checks the master clock, the channels and the front ends, not the product, so the `x310-…` profiles bind an X300 unchanged. Load the X300's image (`uhd_image_loader` picks `usrp_x300_fpga_XG.bit` for it), and record the product (`X300` in the `pp_string`) at the top of `bench-results.md`, so that Phase 8 knows which device its numbers came from.

Results go to `plan/phase7/bench-results.md`, created at the session (GZ-10): the commit, `uhd_config_info --version`, the device's `pp_string`, and for each step the command, its outcome and the numbers it printed. The table at the end of [`plan/spikes/2026-09-26-uhd.md`](../spikes/2026-09-26-uhd.md) is filled from it.

## B0 — Set-up (no RF)

1. **Host.** Rust ≥ 1.85 (`rustup update`); UHD ≥ 4.9 (the OBX; `docker/uhd4.10/Dockerfile` has 4.10 and everything below — Ubuntu's `apt` UHD is older) with headers and its `.pc` file (Ubuntu: `sudo apt install libuhd-dev uhd-host pkg-config`, or the Ettus PPA; a source build under `/usr/local` needs `PKG_CONFIG_PATH` or `UHD_LIB_DIR=/usr/local/lib` to build, and `sudo ldconfig` or `LD_LIBRARY_PATH=/usr/local/lib` to run, since the build adds no rpath, spec 18 UR-35). Check `pkg-config --modversion uhd`.
2. **Network.** X310 10 GbE port 0 at `192.168.40.2`, host NIC `192.168.40.1/24`, MTU 9000; `sudo sysctl -w net.core.rmem_max=33554432 net.core.wmem_max=33554432`. `uhd_find_devices --args addr=192.168.40.2` lists it. An FPGA image that does not match UHD's compatibility number needs `uhd_image_loader --args addr=192.168.40.2` (UHD 4.x's image).
3. **Master clock.** The X310 must run at 200 MHz (spec 18 UR-5 refuses 184.32 MHz for both profiles); if `uhd_usrp_probe` shows another rate, add `master_clock_rate=200e6` to the device string.
4. **Software checks, still without RF.** In a clone at the implemented commit (the branch `worktree-phase7-impl` until it is merged into `main`; `handoff.md` §4 has the session's plan and what the tests implement of each step):
   ```sh
   cargo test --workspace                                   # the software exit criteria
   cargo test -p ezsdr-radio-uhd --features uhd             # the C API tests (uhd_api.rs); no device needed
   cargo test -p ezsdr-radio-uhd --test fake on_one_obx     # B3, B4, B6, B7 on a one-OBX fake (already in the first line)
   cargo build --release -p ezsdr-server --features uhd     # the lab server
   ```
   Record the four outcomes. `docker/uhd4.10/Dockerfile` (or `.devcontainer/`) is a host with all of this installed.

## RF safety (before any step that transmits: B6, B7, B8)

- Cable **TX/RX of the OBX (slot A) to its RX2 through ≥ 30 dB of attenuation** (or the IBFD front end that the owner already uses), or terminate TX/RX in a 50 Ω load for the steps that only need the transmitter to run.
- Every bench profile carries `radio.rf_envelope` with **one narrow allowed band around the test frequency**, `max_gain_db: 0` and `tx_enabled: [true]` (RM-19): a Session command outside it is refused before it reaches the device (§58 #16), which step B7 checks on purpose.
- Transmit gain 0 dB, waveform amplitude ≤ 0.5, a frequency in a band the lab is licensed or shielded for. The steps use the profile's default frequency, 1 GHz on `x310-obx` (RM-5's); if the lab needs another, give `x310-obx` its own default the way `x310-cbx` has one (`profile.rs`, spec 18 UR-9) and change the envelopes below with it, before the session.
- `ezsdr.rf_path` is `cabled` (the class is HardwareInLoop). A Hardware (over-the-air) Run is not part of this session.

## The bench profile

The profile that replaces the Mock one; only the radio binding, the Authority and `ezsdr.rf_path` differ from `crates/ezsdr-acceptance/src/rig.rs`'s `spec_profile` (Vision §59). `<dir>` is a capture directory on the bench PC.

```json
{
  "version": 1,
  "bindings": {
    "radio": {
      "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 1, "patch": 0 } },
      "profile": { "name": "x310-obx", "version": { "major": 0, "minor": 1, "patch": 0 } },
      "selector": { "args": "addr=192.168.40.2", "id": "usrp", "clock_source": "internal", "time_source": "internal" }
    },
    "rec": {
      "module": { "id": "ezsdr.sink.capture", "version": { "major": 1, "minor": 2, "patch": 0 } },
      "selector": { "dir": "<dir>" }
    }
  },
  "authority": "radio",
  "placements": {
    "links": [{
      "link": { "id": "ezsdr.link.host", "version": { "major": 1, "minor": 0, "patch": 0 } },
      "from": { "component": "radio", "port": "rx" },
      "to": { "component": "rec", "port": "in" }
    }]
  },
  "environment": {
    "ezsdr.time": { "class": "hardware_in_loop", "start_lead_ns": 2000000000 },
    "ezsdr.rf_path": { "path": "cabled" },
    "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 999000000, "hi_hz": 1001000000 }], "max_gain_db": 0, "tx_enabled": [true] }
  }
}
```

Every hardware test takes the device string from `EZSDR_UHD_ARGS` and writes its profile itself. The Python step B7 reads the **Session profile** from the file `EZSDR_PROFILE` names (VE-5); save this one as `bench-session.json` (the Spec profile above with a `feed` on `rec`, as SB-22c requires of a Session):

```json
{
  "version": 1,
  "bindings": {
    "radio": {
      "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 1, "patch": 0 } },
      "profile": { "name": "x310-obx", "version": { "major": 0, "minor": 1, "patch": 0 } },
      "selector": { "args": "addr=192.168.40.2", "id": "usrp", "clock_source": "internal", "time_source": "internal" }
    },
    "rec": {
      "module": { "id": "ezsdr.sink.capture", "version": { "major": 1, "minor": 2, "patch": 0 } },
      "selector": { "dir": "<dir>" },
      "feed": { "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 64 }
    }
  },
  "authority": "radio",
  "placements": {
    "links": [{
      "link": { "id": "ezsdr.link.host", "version": { "major": 1, "minor": 0, "patch": 0 } },
      "from": { "component": "radio", "port": "rx" },
      "to": { "component": "rec", "port": "in" }
    }]
  },
  "environment": {
    "ezsdr.time": { "class": "hardware_in_loop", "start_lead_ns": 2000000000 },
    "ezsdr.rf_path": { "path": "cabled" },
    "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 999000000, "hi_hz": 1001000000 }], "max_gain_db": 0, "tx_enabled": [true] }
  }
}
```

## The steps

Run each with `EZSDR_UHD_ARGS=addr=192.168.40.2 cargo test --release -p ezsdr-radio-uhd --features uhd --test hardware <name> -- --ignored --nocapture`. The order matters: each step checks what the next relies on.

| # | Test (spec 18 UR-34) | What it does | Passes when | Records (for `bench-results.md` and Phase 8) |
|---|---|---|---|---|
| B1 | `hw_b1_probe` | `open`, `describe`, channel counts, `ref_locked`, three time reads 100 ms apart, and every channel's transmit frequency and gain as the device reports them (before any step transmits: what a Session would inherit if nothing configured the transmitter, spec 18 UR-25) | the device opens at 200 MHz; the profile is `x310-obx` (the test fails otherwise), with at least its 1 + 1 channels (UHD reports 2 + 2: slot B empty is its unknown board); time advances by 20 000 000 ± 1 % ticks per 100 ms | `pp_string`, UHD version, sensors, the front ends' names, the transmit settings found |
| B2 | `hw_b2_authority` | builds `DeviceAuthority`; schedules 100 wakeups 10 ms apart and measures how late each `next_wakeup` returns in host time; reads `relations()`; re-anchors for 10 s and records the anchor's drift | median lateness < 1 ms, maximum < 25 ms (the 20 ms nap plus scheduling); both relations present | the lateness distribution; the relations' uncertainties; the observed device/host drift (UR-8's INFERRED bound) |
| B3 | `hw_b3_receive_at_t0` | `experiments::receive(1, 1e6, f, Some(10_000))` at the bench frequency `f` under the bench profile (no RF needed) | `Completed`; the capture's first continuity map starts at receive sample 0 with no gap; `timing` shows the first block at index 0; `applied` shows the rate equal to the claim | the frequency read-back difference (K13: expected < 0.05 Hz); the first block's host delay after `start` |
| B4 | `hw_b4_capture_at_a_sample_index` | a Session capture of 10 000 samples `at` receive sample 50 000 (v3 behaviour 2) | the artifact's first sample is index 50 000 | — |
| B5 | `hw_b5_overflow` | a receive Run with `UhdRadio::with_rx_stall(500 ms, 300 ms)` at 10 Msps; if no overflow occurs, again with the stall at 1 s and then 2 s (the host's socket buffer, `net.core.rmem_max`, may absorb 300 ms of `sc16` at 10 Msps, about 12 MB) | `radio.RX_OVERFLOW { cause: overrun }` delivered; the capture spanning it has an `overflow_restart` gap whose `lost` equals the block's time jump (§58 #6) | the stall that first overflowed and the socket buffer sizes; the measured restart gap (MR-3 claims 50 ms, VERIFIED from UHD's source) |
| B6 | `hw_b6_txrx_and_repeat` | with the loopback cable: a timed burst and a timed capture on one device (v3 behaviour 3); then a 3 s `start_repeat` of a 1 000-sample waveform, captured | the burst's correlation peak is found; the repeat's capture shows the waveform back to back across every wrap; no `TX_UNDERFLOW` | the transmit-to-receive delay in samples (MR-3 claims 45, INFERRED); the repeat's `wraps` |
| B7 | Python, not a test | `EZSDR_SERVER=target/release/ezsdr-server EZSDR_PROFILE=<session profile> python3 python/examples/minimal.py` (Vision §3's snippet, unchanged); then `python3 python/examples/bench_loopback.py` (`tx.repeat(x)`; `y = sdr.rx.capture(N, at=sdr.after(0.05))`; `t = sdr.sleep(0.1)` and `z = sdr.rx.capture(N, at=t)`; then `sdr.rx.sample_rate = 19.5e6`; then a retune outside the RF envelope) | both run; `y` holds the waveform and starts at its instant; no `TIME_ERROR` in the Manifest (spike K6); the rate is coerced to 20 Msps and starts a new SampleClock (UR-25); the out-of-envelope retune raises `ezsdr.Rejected` naming `radio.rf_envelope` and is logged (§58 #16) | the Manifest paths; whether `z` started at `t` (EA-17's INFERRED case: does a local request outrun the device's receive latency?) |
| B8 | `hw_b8_leads` | bursts at leads of 10, 5, 3, 2, 1.5, 1, 0.5 ms from their receipt; timed retunes at the same leads; `cold` rate changes at restart leads of 100, 50, 25, 10 ms; a repeat stopped by `Stop`, measuring where its record ends; repeats at in-flight windows of 10, 5, 3, 2 ms, watching for `TX_UNDERFLOW`; a burst preempting a running repeat at leads of 20, 10, 5 ms; 1 to 32 timed retunes queued ahead, timing each `apply` call; a timed stop of the receive stream | — (a measurement, not a pass/fail step) | the device lead (the least lead with no `late_at_device`), the delivery allowance (receipt − dispatch, from `timing`), the restart lead, the least in-flight window without underflow, the preemption bound, how many timed OBX tunes fit before `apply` blocks (UR-24's queue depth), the release window's margin, the transmit end after `Stop`, whether a timed receive stop is honoured (UR-25's INFERRED point) |
| B9 | `hw_b9_unplug` (manual) and `hw_b9_usrp2_probe` | during a receive Run, and again during a transmit-only Session with nothing sent, unplug the 10 GbE cable; separately, run the steps B3–B7 once more and check that no `DEVICE_LOST` appears; then `EZSDR_UHD_ARGS=addr=192.168.10.2 … hw_b9_usrp2_probe` on the USRP2, which opens it, prints `describe()` and builds a `UhdRadio` from a binding of the profile its front end names | `DEVICE_LOST` within about 1 s in both unplugged Runs, each ending `Stopped { policy { DEVICE_LOST } }` with its Manifest written; no false `DEVICE_LOST` in the rerun; the USRP2 opens and `from_binding` refuses it by UR-5 (100 MHz master clock) | the UHD error each unplug produced (UR-29's INFERRED list, `UHD_ERROR_RUNTIME`'s false positives); the USRP2's `pp_string` (its daughterboard, for a future profile) |

Criterion 9 of the overview's §10 is B0–B8 passing; B8 and B9 are recorded whatever they show.

## If a step fails

A failure on the bench is a finding, not a bug to patch at the bench: record it in `bench-results.md` with its output, and fix it in `main` with a test on `FakeDevice` that reproduces it before the step is run again. Common causes the spike anticipated: a UHD/FPGA compatibility mismatch (`uhd_image_loader`), a master clock other than 200 MHz (B0.3), socket buffers too small for the rate (UHD prints a warning at start), and a USRP2 that UHD 4.x no longer drives (not verified on this build).
