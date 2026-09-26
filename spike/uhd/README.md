# UHD spike (branch `spike/uhd`, not for `main`)

How far the Phase 1–3 Kernel carries a real USRP. `ezsdr.radio.uhd` 0.1.0 is one
Module with two roles — a hardware Provider of the `radio` Vocabulary (never stepped,
MA-15) and a device-paced Time Authority (`Pacing::Device`) — run by the unmodified
coordinator except for one line (K1 in [FINDINGS.md](FINDINGS.md)).

The Specs are the acceptance crate's own (`experiments::receive`, `transmit`,
`with_timed_capture`), with only frequency and gain set. Only the BindingProfile
differs from the Mock tests — which is Vision §59's claim.

## Build and offline check (no hardware)

```sh
cd spike/uhd
cargo test                      # 6 tests on the wall-clock fake device (~2 s)
cargo run --bin uhd-spike -- rx fake
```

libuhd is found through `UHD_LIB_DIR`, else `pkg-config --variable=libdir uhd`,
else `/opt/homebrew/lib`. Tested against UHD 4.10.0 (Homebrew, arm64).

## At the bench

Order matters: each step checks what the next relies on. Every Run writes
`out/<cmd>-<time>/{spec,profile,manifest}.json` and the capture file, and prints a
summary (termination, the Provider's sections, events, continuity).

`ARGS` is the UHD device string: `addr=192.168.40.2` (X310 10 GbE port 0),
`addr=192.168.10.2` (X310 1 GbE, or USRP2/N2x0 default), or `type=x300`.

| # | Command | Checks | Look at |
|---|---|---|---|
| 0 | `cargo run --release --bin uhd-spike -- find` | FFI + discovery | device listed |
| 1 | `… probe ARGS` | open, master clock, time advancing, rate coercion | `time advances` ≈ mcr/10; `rx rate 19.5e6 →` |
| 2 | `… rx ARGS --freq 2.4e9 --gain 10` | M1: T0 start, timed capture (§61 v3 behaviour 2) | `timing.first_rx_block.k == 0`; continuity starts at 10000; `coercion` rows (claimed vs device) |
| 3 | `… session-rx ARGS --at 50000` | Session capture at a requested sample index | "starts at rx sample 50000 (asked 50000)" |
| 4 | `… overflow ARGS --rate 10e6` | M2: stalled host → `O` → gap + flags (§58 #6) | `radio.RX_OVERFLOW` event, `gaps` with `overflow_restart`, `restart_gap_ns` |
| 5 | `… txrx ARGS --tx-gain 0 --gain 10` | M3: timed burst + timed capture on one device (§61 v3 behaviour 3) | correlation peak → measured TX→RX delay (x310-like claims 45 samples, INFERRED) |
| 6 | `… repeat ARGS --seconds 3` | M5: continuous repeat, wrap (§61 v3 behaviour 1) | `bursts[0].wraps`, any `underflow` in `async` |
| 7 | `… loopback ARGS` | §57: `tx.repeat(x); rx.capture(N)` as a Session | log entries admitted; `TIME_ERROR send_asap` (finding K6) |
| 8 | `… leads ARGS` | device floor for start/TX lead, Provider checks off | at which lead `rx_errors` shows a late command and `async` a `time_error` |

**RF safety for 5–7**: TX/RX and RX2 of one UBX cabled through ≥ 30 dB attenuation
(or the existing IBFD front end), `--tx-gain 0`, amplitude 0.3–0.5. Default frequency
1 GHz; use `--freq` for the band you are licensed/shielded for.

Common options: `--rate 1e6 --freq 1e9 --gain 0 --tx-gain 0 --grid x310-like|ideal
--lead-ms 500 --rf-path cabled|over_the_air --clock internal|external --time
internal|external --out DIR`. With `--time external` the Authority sets time with
`set_time_unknown_pps` (needs PPS).

- **X310 at 184.32 MHz master clock**: the `x310-like` grid assumes 200 MHz; expect a
  refusal or coercion rows that disagree — that is a finding, not a bug to fix here.
- **USRP2/N2x0**: 100 MHz master clock; use `--grid ideal` (the x310-like rate grid is
  200 MHz/N). UHD 4.x still ships the `usrp2` driver (not verified on this build).
- If `find` sees nothing: host NIC on 192.168.40.1/24 (10 GbE, MTU 9000) or
  192.168.10.1/24 (1 GbE); `uhd_find_devices --args addr=…`; an FPGA image
  mismatch needs `uhd_image_loader` for UHD 4.10.
