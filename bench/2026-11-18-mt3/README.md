# 2026-11-18: the receive start lead at 100 MS/s and maintenance table 3 cells C1–C6 on both boards

Code `5b8f2b32` (#1's S5 and C1–C6) and `72a5eea1` (C5's moved grid on #1, every run on #2; the two differ only in C5's grid). #1 X300 + OBX (slot A) + UBX (slot B) 14:58–15:09, then the UBX moved back to #2's slot A, #2 X300 + UBX 15:18–15:24. Every used slot TX/RX → 30 dB → RX2, 1 GHz, gain 0 dB, tone 100 kHz at 0.4. No power cycle, quiet host. Conditions: [conditions.json](conditions.json). Rows: [results/decisions.tsv](results/decisions.tsv) (S5 decisions and stage lines, MT3 cell, boundary, marginal and reading lines), [results/verdicts.tsv](results/verdicts.tsv) (per-trial verdicts). Raw logs: [raw.txt](raw.txt). Procedure: [plan/phase8/bench.md](../../plan/phase8/bench.md), the last two sections.

| Item | #1 (OBX slot A) | #2 (UBX slot A) |
|---|---|---|
| S5 receive start lead, 1 channel, 100 trials a rung | 10 ms clean in 4 of 4 conditions; smallest clean rung: restart-same 4 ms (10 MS/s), 5 ms (100 MS/s; 4 ms 73/100); restart-retune 7.5 ms at both rates; 2 000 trials all started; `lead_left_ns` medians of the first and last 10 trials of each rung within 0.2 ms | the same rungs (restart-same 100 MS/s 4 ms 88/100); 2 000 started; first/last-10 medians within 45 µs |
| C1 chunked late burst, 1 and 10 MS/s × 10 | PASS: nothing heard in the late burst's interval, no BurstAck, TimeErrors | PASS |
| C2 100 and 200 MS/s × 10 | INCONCLUSIVE 20/20: the late burst's last send was handed 0.9–1.5 ms after its own time (the tool assumed the host outruns real time at these rates); no trial played or acknowledged the late burst | not run |
| C3 first send 32 samples, 1 and 10 MS/s × 10 | PASS | PASS |
| C4 cascade boundary, 1 and 10 MS/s | PASS; the device's own boundary: B dropped at δ ≤ 19 µs, played at δ ≥ 21 µs, mixed at 20 µs (9/1 at 1 MS/s, 6/4 at 10 MS/s); the fake's limit is 256 µs (1 MS/s) and 242 µs (10 MS/s) | PASS; the same bracket (20 µs: 8/2, 3/7) |
| C6 held tail starves, 1 and 10 MS/s × 10 | PASS: an Underflow at the head's end, then a TimeError, no BurstAck | PASS |
| C5 two channels, 1 MS/s (slot B confirmed) | grid −200…200 µs: clear PASS, every marginal trial dropped on both channels (the first send handed 128–729 µs after its first sample's time): INCONCLUSIVE. Grid 0…1500 µs: straddled (0 and 200 µs 0/10 played, 300–500 µs mixed, ≥ 600 µs 9–10/10): **SPLIT in 2 of 90 trials** (500 µs #5, 600 µs #0): slot A played (its BurstAck, no TimeError, the tone heard), slot B dropped (370–371 TimeErrors, no ACK, only the control heard); the first send was handed 1.3 and 2.5 µs after its time | not runnable (slot B empty) |

Decisions it fed (owner, 2026-11-18):
- The receive start lead `rx_start_lead_ns` stays 10 ms: now measured at 100 MS/s on an OBX and a UBX (#1, #2), 1 GHz, slot A, one channel; restart-retune needs 7.5 ms.
- RM-11: `late_at_device` means nothing of the burst was transmitted on at least one channel; on a stream of several channels another channel may have played it (C5, #1, at these conditions). KL-45 records it.
- The late-device-burst rule (whole burst dropped, the drain ends about 20 µs after `t0 + S × per`, inside the fake's pad) holds on both boards at 1 and 10 MS/s with the Module's chunk shape and a 32-sample first send (C1, C3, C4, C6). Not measured: 100 and 200 MS/s (C2: the tool's premise failed; to be redesigned with bt2), C5 on #2, C7 (bt2).
