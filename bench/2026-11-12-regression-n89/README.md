# 2026-11-12: T1 regression, the Stop/switch set, and #89 on the X300s

Code `1231462` (E2b `bdf3493` and #89 `fe86629` included), both boards, #2 (UBX) then #1 (OBX), 09:53–11:05, no power cycle. #2's CBX was removed by the owner before the session (slot B empty). Conditions: [conditions.json](conditions.json). Rows: [results/verdicts.tsv](results/verdicts.tsv). Raw logs: [raw.txt](raw.txt). Procedure: [plan/phase8/bench.md](../../plan/phase8/bench.md) "after note 47 stage 5" (b), "after step F" (d)/(e)/(g) and "#89 の後".

| Test | #2 UBX | #1 OBX |
|---|---|---|
| (b) T1 regression, 9 tests (incl. step F's end+3 refused / end+4 plays, B7's retune) | 9/9 PASS, no TIME_ERROR, no TX_UNDERFLOW | 9/9 PASS (same) |
| `hw_n47s5_held_tail_and_stop` (J(e), c+5, R5-m5 at 6 rates) | PASS (the Session row INCONCLUSIVE, as before) | PASS (same) |
| `hw_n47s5_repeat_switch`, untimed and timed | PASS | PASS |
| `hw_b8_stop_end`, 3 trials | PASS | PASS |

#89 (wiring B: #1 OBX transmits, #2 UBX is the independent receiver):

| Run | Result |
|---|---|
| (i-a) `sleep` at `8dd1016` (before #89) | C1 FAIL (server still running 15 s after the drop), C2 FAIL (tone still on at +14 s), C3 PASS — the hole reproduces on the old commit |
| (i-b) `sleep` at `1231462` | C1 PASS (server exited at +0 s, Manifest `client_disconnect` under an attached Lease), C2 PASS (quiet at +0 s), C3 PASS |
| (ii) `kill` at `1231462` (E9) | after `docker kill` of the server container: last tone +81 ms, first quiet +608 ms (bracket 527 ms) from `FinishedAt`; C3 PASS |
| (i-a)'s emergency stop (E9's first data point) | last tone +54 ms, first quiet +582 ms (bracket 528 ms) |

Decisions it fed:
- #89 closed: its closing conditions hold ((i-a) FAIL, (i-b) C1–C3 PASS).
- E9: a dead server stops the device by starvation within about 0.6 s (conditions: 1 MS/s, 100 kHz tone, #1 OBX, one run per mode). The owner adopted A1 (the server's fail-stop backstop) on this evidence (2026-11-12).
- E2b and #95's time arithmetic behave on hardware as their FakeDevice rehearsals.
