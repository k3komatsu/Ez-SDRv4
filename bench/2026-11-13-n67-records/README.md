# 2026-11-13: #67's stop-before-origin probe, receive leads and zeros, C2's switch, and the record suite unloaded and loaded

Code `3f1f87f`, both boards (#2 UBX and #1 OBX, slot B empty on both), 11:31–13:04, no power cycle. Conditions: [conditions.json](conditions.json). Rows: [results/verdicts.tsv](results/verdicts.tsv) and [results/measurements/](results/measurements/). Raw logs: [raw.txt](raw.txt).

| Item | Result |
|---|---|
| 1 probe | both boards fine; `ref_locked` true on both (#1 was false on 11-09) |
| 2 #67 (`hw_67_stop_before_origin`, 1/10/100 MS/s × 10) | `stop_before_origin` 30/30 PASS per board (the untimed rx_stop cancels the queued timed start; no block of it arrives); `close_before_origin` 30/30 FAIL per board (close/reopen does not cancel it: the old start's blocks arrive on the reopened streamer at the origin); `stop_after_origin` (control) 30/30 PASS |
| 3 UR-17, restart lead (#2) | distributions in the logs; the restart lead stays at +0 ticks down to 1 ms, warm and cold |
| 4 start lead, restart fill (#2) | smallest start lead 5 ms; restart fill zeros at 1/2/5/10/25/50/100/200 MS/s: 14/10/14/11/13/9–10/3–4/0 (5 trials each, the same as earlier sessions) |
| 5 C2 repeat switch | untimed and timed PASS on both boards (applied, no seam) |

Record suite on #1 OBX (`hw_measurements_record`; R1 unloaded 2154 s, R2 under the `h4` load 2409 s, CPU busy 100 %). Both wrote `measurement.json`; both fail only the profile comparison (profile 1 GB/s looser than measured; stage 4 material).

| Quantity | R1 (no load) | R2 (h4 load) |
|---|---|---|
| receive throughput (60 s stages) | to 200 MS/s, no overflow (800 MB/s) | to 50 MS/s (200 MB/s); 100 and 200 MS/s overflow (DeviceFailure); reopened 100 MS/s passes |
| transmit throughput | to 100 MS/s (400 MB/s); 200 MS/s underflows | the same |
| #56 Stop lag, submit → rx_stop start / end (20) | median 0.98 / 1.59 ms, max 1.93 / 2.89 ms | median 0.20 / 1.83 ms, max 1.36 / 3.00 ms |
| #56 Stop between starts (3) | clean (no refusal, no TIME_ERROR) | clean |
| H1 late 20 000-sample burst (late 1/5/10 ms × 3) | all-timed (9): nothing heard, no BurstAck, 365–368 TIME_ERROR reports delivered per burst (a count of delivered reports, not of packets: the 32-entry error FIFO loses most); timed head + untimed tail (9): 19 544–19 545 of 20 000 heard, 39–40 TIME_ERROR reports, one BurstAck — the FPGA drops late packets one by one, at one sample per tick, and the DUC carries the first time to the burst's end, so an all-timed burst is dropped whole | all-timed (9): nothing heard, no BurstAck, 313–454 TIME_ERROR reports; timed head + untimed tail (9): 19 544–19 545 heard, 36–39 TIME_ERROR reports, one BurstAck (the same outcome; rows in the raw archive) |
| H2 10 ms starvation mid-burst (3) | Underflow at the head's end (−6 ticks); the untimed tail plays on arrival (10 244–10 276 samples after the head's start), one TIME_ERROR at its start, BurstAck at the end | — |
| H4 transmit headroom proxy, 10 MS/s rx+tx, 2 s | median 5.93 ms, min 4.61 ms, 0 underflow | median 5.90 ms, min 3.92 ms, 0 underflow |
| H5 held last sample | 600 bursts, 0 TIME_ERROR, 0 underflow | PASS |
| T6 stop end (3) | PASS (−7 200…12 000 ticks of 30 000) | PASS (3 000…12 800) |

Notes:
- The `hw_56_*` cleanup drains a closed receive streamer about 184 000 times ("UR-3: the receive streamer is not open"): a test-tool busy loop, no effect on the verdicts; to fix.
- The throughput values hold for this host and #1 OBX, one channel; under load the receive limit is the host, not the device (INFERRED).

Decisions it fed:
- #67: a fix must send an untimed rx_stop before close (close/reopen alone lets the old timed start's data arrive at the origin).
- H1: input to maintenance table 3. The X310 drops a late all-timed device burst whole (it drops packet by packet, at one sample per tick, and the DUC carries the first time to the burst's end); the fake had dropped only the late packet since D9b-1a (`80dd2f6`) and table 3 makes it drain the whole device burst. The Module's chunked shape, other rates, board #2 and a burst behind a late one are not measured (bench.md, cells C1–C7).
- Stage 4: the profile's `rx_bytes_per_s` / `tx_bytes_per_s` (1e9) are looser than measured (8e8 / 4e8 unloaded; 2e8 receive under load).
