# sync_check and tx_packet_probe

Bench programs for the 2026-11-27 session (8b D-1/D-4 checks; Phase 8 H3). C11, UHD C API only. **Neither transmits** (`grep -n 'tx_' sync_check.c` and `grep -n 'send\|tx_metadata' tx_packet_probe.c` return nothing). Procedure and expected results: the 8b step-0 draft, section 8.

## Build (binary outside the tree)

```sh
S=$PWD/plan/phase8/tools/sync_check; L=~/.cache/ezsdr-bench/2026-11-27-8b0; mkdir -p "$L"
for p in sync_check tx_packet_probe; do
  docker run --rm --user "$(id -u):$(id -g)" -v "$S:/src:ro" -v "$L:/out" ezsdr-v4-dev:uhd4.10 \
    cc -std=c11 -Wall -Wextra -Werror -O2 /src/$p.c -I/usr/local/include -L/usr/local/lib -Wl,-rpath,/usr/local/lib -luhd -lpthread -lm -o /out/$p
done
sc() { docker run --rm --network=host --user "$(id -u):$(id -g)" -v "$L:/out" ezsdr-v4-dev:uhd4.10 /out/sync_check "$@"; }
```

## sync_check MODE [--run N] [options] ARGS

`A`, `A0`, `A1` are UHD device args such as `addr=192.168.40.36`.

| MODE | Command | Line (stage) |
|---|---|---|
| probe | `sc probe --run 1 A` | `S0` |
| separate | `sc separate [--order 01\|10] [--pps-out on\|off] A0 A1` | `S3`; `S3d` with `--pps-out on`, `S4b` with `off`, `S4c` with `--order 10` |
| reopen-source | `sc reopen-source A0 A1` | `S5a` |
| outputs-off | `sc outputs-off A0` | `S5b-off` |
| follower-first | `sc follower-first A1` | `S5b` |
| compound | `sc compound "addr0=..,addr1=..[,clock_source1=external,...]"` | `S2` |
| align | `sc align --freq 1e9 --rate 200e6 --samps 65536 --gain 0 A0 A1` | `S6` |
| selftest | `sc selftest` (no device) | `selftest ok` |

Output: one line on stdout, `STAGE run=N key=value ...` (lists are `,`-separated; times in ms from process start; `last_pps`, `time_now`, `incr*`, `t_start` in ticks of 200 MHz; `tnow_delta_ns` the largest |time_now(h) - time_now(h0)|). Text values are in `"..."`. A failed UHD call prints `STAGE run=N verdict=ERROR error="<call>: <UHD message>"`.

`align`: `lag_samples` > 0 means device 1's copy of the signal is later than device 0's; `lag_ticks` = `lag_samples` x 200e6 / rate; `peak_db` and `second_peak_db` are the normalised correlation peak and the best peak more than one sample away (0 dB = identical). The search is +-64 samples.

`separate` lines also carry `order=`, `void_attempts` (attempts whose arming straddled an edge), `set_s` (the nonce time set, 1000 s beyond every handle's time; the check requires every handle's `time_now` in [set_s, set_s + 1.1 s) after the edge) and `missed` (per handle: expected edges minus edges it showed). `incr0` and `incr1` are the differences between each handle's own consecutive `last_pps` values, polled every ~1 ms over 3 leader edges + 0.5 s. A run prints exactly one verdict line: the watchdog prints its ERROR only if the main path has not claimed the line. Every mode has a watchdog (`--timeout S`, default 300) that prints `STAGE run=N verdict=ERROR error="timeout"` and exits 2.

Exit status: `PASS` and `INFO` 0, `FAIL` and `REFUSED` 1, `ERROR` 2, `INCONCLUSIVE` 3 (every time-setting attempt was void because an edge crossed the issuing; says nothing about the devices). A bad MODE or missing arguments print usage and exit 2.

## tx_packet_probe ARGS [RATE ...]

`docker run --rm --network=host --user "$(id -u):$(id -g)" -v "$L:/out" ezsdr-v4-dev:uhd4.10 /out/tx_packet_probe addr=192.168.40.36` (default rates 1e6 10e6 50e6 100e6 200e6; 1 and 2 channels, 2 skipped with a `skipped=` line when the device lacks two TX or RX channels).

Lines: `H3 rate= channels= cpu=fc32 otw=sc16 max_num_samps=` (TX streamer) and `H3rx ...` (RX streamer, same args). It creates and frees streamers only. Exit 0; 2 with one `H3 verdict=ERROR error="..."` line on failure.
