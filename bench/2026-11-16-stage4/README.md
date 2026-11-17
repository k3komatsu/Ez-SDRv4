# 2026-11-16: stage-4 cells on both boards, T200 H0, and the T200 isolation (v3, 1 M samples per send)

Code `8d876903` (evening bare runs `90d55d0c`), #2 X300 + UBX (slot A) 08:54–11:52, then the UBX moved to #1's slot B, #1 X300 + OBX (slot A) + UBX (slot B) until 18:47, and the T200 isolation on #1 20:03–20:36. One power cycle (#1, after the SIGSEGV at 15:44). Conditions, invalid logs and host load: [conditions.json](conditions.json). Rows: [results/decisions.tsv](results/decisions.tsv) (every decision and summary line), [results/verdicts.tsv](results/verdicts.tsv), [results/trials.jsonl](results/trials.jsonl) (S2 and S4 trial rows). Raw logs: [raw.txt](raw.txt).

The host was loaded by another project's simulation from about 14:08 to about 16:20: #1's first S2 run is invalid (every cell failed, receive included) and was rerun on a quiet host. The chains then checked the load before each step.

| Item | #2 (UBX slot A) | #1 (OBX slot A) |
|---|---|---|
| #67 (`hw_67_*`, 1/10/100 MS/s × 10) | `stop_before_origin`, `stop_after_origin`, `close_then_stop_before_origin` 30/30 PASS; `close_before_origin` 30/30 FAIL (expected) | the same |
| rxzeros (RXZ) | direction D does not hold (a class wider than ±1: D50 5–7, D250 5–6 zeros); a fresh open is within ±1 of a restart; gain moves a limit by more than 2; precursor 0.078 | D does not hold (D50 and D250 6–7, D10 4–5); fresh open within ±1; precursor 0.081 |
| PN (RXZPN) | delay 0 at 1–100 MS/s; 200 MS/s INCONCLUSIVE (7 of 10) | 200 MS/s INCONCLUSIVE (6 of 10) |
| T3 (P75) | a INCONCLUSIVE (control not clean); b an ACK at an old burst's end shows on the new streamer (match by tick); c the ACK still readable 150 ms after T3 | the same |
| OFS | one channel: every flush acknowledged within 100 ms | one and two channels (slot B confirmed): every flush acknowledged within 100 ms → enable #75 (3) for 1, 2 and 10 MS/s, 1 and 2 channels, #1 |
| S5 command lead | 5 ms clean everywhere; 3.5 ms clean at 1/10/100 MS/s | the same |
| S5 receive start lead (1 channel) | 10 ms clean at 10 MS/s (from 5–7.5 ms), not at 100 MS/s (no rung clean) | the same |
| S5 two channels | not runnable (the pass opens a slot-A-only device: a test defect) | the same |
| S2 receive 100 / 200 MS/s, 60 s × 10 | 10/10, 10/10 (no overflow) | 10/10, 10/10 |
| S2 transmit 100 / 200 MS/s, 60 s × 10 | 8/10, 0/10 (8/10 clean in the first 10 s) | 8/10, 2/10 (9/10 in the first 10 s) |
| S2 duplex rx200+tx100 / rx100+tx100, × 5 | 3/5, 4/5 | 2/5, 3/5 — every failure a transmit underflow; no receive overflow in any duplex trial |
| S4 overflow gap, 20 trials a rate | 1, 10, 25, 50, 100 MS/s 20/20 Success (one overflow each; restart 69–78 ms after resume); 200 MS/s: the capture sink's cleanup ran past RS-8a's 5 s (EZSDR-INFRA, then SIGSEGV), stall sweep not run | the same (restart 69–87 ms) |
| T200 bare UHD, 200 MS/s, 300 s × 3 | underflow 33, 34, 28 | underflow 26, 19, 32 (no timeout, sequence or time error) |
| T200 Module probe (20 000-sample repeat) | DeviceFailure × 3; samples handed 5.998e10, 1.88e10, 0.27e9 of 6.0e10 | DeviceFailure × 3; 5.997e10 each (13–14 underflows) |
| T200 record shape, 1 000 / 20 000 samples | 0/3 (underflows 3, 5, 7) / 0/3 (4, 5, 2) | 0/3 (9, 13, 11) / 0/3 (14, 9, 8) |

T200 isolation on #1 (evening, quiet host checked before each run):

| Sender | samples per `send` | underflows in 300 s |
|---|---|---|
| bare UHD (H0, above) | 1 996 | 26, 19, 32 |
| bare UHD | 1 000 000 | 136 (right after the backup and a load of 5), 22, 21 |
| v3 v3.0.28 on UHD 4.10, MultiUSRP + CyclicTX | 1 000 000 (whole loop per call) | about 49, 63, 54 (UHD `U` characters; v3 counts no async events) |

Notes:
- "DeviceFailure" in S2 and T200 is the test's class for a trial with any underflow; the Run itself went on. The Module's repeat resumed after each underflow (UR-22) on #1; on #2 two probe trials stopped handing samples right after two underflows, which the stutter bound b′ explains (INFERRED: the Manifests were not kept). Issue #106.
- The `TimeError` reports that follow each underflow are the packets already handed when the device ran dry, reported late.
- UHD 4.10 caps `send_frame_size=9000` at 8000 bytes on this X300 HG image (1 996 samples a packet either way, VERIFIED by opening #1). The stream endpoint's ingress buffer is 32 768 × 8 bytes = 256 KB, about 0.33 ms at 200 MS/s (`x300_HG_rfnoc_image_core.yml`; the word unit INFERRED from the Verilog's `INGRESS_BUFF_SIZE`).
- Bare UHD already ran at the highest real-time priority. Host settings H1 (C-states) and H2 (IRQ affinity) were not run: the owner ended the T200 investigation (below).

Decisions it fed:
- Transmit at 200 MS/s from the host is not sustainable on this host with any sender (bare UHD, v3, the Module); 100 MS/s holds a 60 s trial in 8 of 10. The owner (2026-11-16): stop investigating, use the device's Replay (8c) for a continuous 200 MS/s repeat; KL-5 says so.
- #106 (owner, 2026-11-16): drop the stutter bound b′ (a repeat resumes after every underflow) and bound the Module's per-Run records.
- Open for stage 4a and later: rxzeros direction D fails on both boards (the rule says B), the receive start lead at 100 MS/s, #75 (3) enable conditions, the S5 two-channel pass, S4 at 200 MS/s against RS-8a.
