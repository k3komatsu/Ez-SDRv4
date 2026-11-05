# Dispositions — Sol pre-check of the bench-preparation brief (2026-11-05)

Source: sol-precheck.log (BRIEF NEEDS CHANGES). T1 OK, T3 OK (with assertions kept), T2/T4/T5/T6/T7 CHANGE.

| Item | Disposition |
|---|---|
| T1 kept-raw ≠ approved to run (queue-depth probe) | adopted |
| T2 stream-scoped Stop, untimed values + timed start, configure read-back, first-1-ms tone | adopted |
| T3 keep origin / GAP_BEFORE / boundary-crossing retune; 1 GHz boundary | adopted |
| T4 RX drain to quiescence, device metadata, generalized tone helpers, fixed TX source | adopted |
| T4/T5 TX flush init, EOB ≥ 2, ACK-confirmed end, calibrated loopback delay, booking lead | adopted |
| T5 −5 ms as owner-approved disruptive control, issue bracket, power/truncation, gain ≤ 0 | adopted |
| T6 direction-specific coverage, serialized trials, antenna with streams stopped, censored buffer outcomes, recv_buff_size semantics | adopted |
| T7 replace P4/L1 expectations, P7 historical, both boards, env vars, per-block time bounds, no runtime .stream | adopted |
| Brief-wide: use remote_mutate.py instead of throwaway-copy probes | rejected: AGENTS §9 allows a debug copy outside the repository; it forbids mutate.py and mutants in the main tree. remote_mutate.py runs at the orchestrator's gate |

# Dispositions — Sol code review, Opus re-checks (2026-11-05)

| Finding | Disposition |
|---|---|
| Sol 1 (P1) timed transmit campaign can leave DUC commands queued | adopted differently: the timed transmit trial is removed (reduce paths), T6 times receive only |
| Sol 2 B expected at 200 kHz | fixed (100 kHz) |
| Sol 3 partial final send / 1-sample EOB | fixed (`n47::burst_send`, ≥ 2-sample closing send) |
| Sol 4 −5 ms control issued after A's ACK | fixed (issued before; classified by the device-time bracket) |
| Sol 5 gain −10 dB impossible | adopted differently: the ≤ 0 dB rule was the orchestrator's; owner allows transmit gain to max with the attenuator confirmed → T5 0 → +6 dB, T6 0 ↔ +1 dB with no samples |
| Sol 6 T2 baseline from a stopped interval | fixed (floor after the repeat stops) |
| Sol 7 first window accepts a missing prefix | fixed (coverage and every sub-window) |
| Sol 8 T4 receive source expires | fixed (source restarted per trial) |
| Sol 9 drain quiet rule | fixed (`Drain { read, quiet }`, failed drain → INCONCLUSIVE) |
| Sol 10 buffer trial without progress | fixed (incomplete trials excluded) |
| Sol 11 origin compared with the device call | fixed (booking / registered SampleClock) |
| Sol 12 companion rate not requested | fixed (requested and read back) |
| Sol 13 lead_left missing on success | fixed (from the configure's `applied` rows) |
| Sol 14 leftovers, duplicate tone generator | fixed |
| Opus 1 N1 ACK ±1 tick | fixed: `[e − 2 samples, e + 1]`, outside INCONCLUSIVE, delta printed |
| Opus 1 N2 lost coverage then untimed apply | fixed: rx_stop then panic with a power-cycle message |
| Opus 1 N3 a–f | fixed |
| Opus 2 three P3 (B ACK missing → FAIL; A duplicate counted early; move tick frame) | fixed by the orchestrator, row N47B-41 |
