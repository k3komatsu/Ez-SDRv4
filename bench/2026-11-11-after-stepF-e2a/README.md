# 2026-11-11: step F's front and note 49 E2a on both X300s

The 30-minute set of [plan/phase8/bench.md](../../plan/phase8/bench.md) "after step F" (owner, 2026-11-11): (c) the front, (d) the held tail and the Stop region, and the hardware tests whose expectations depend on E2a's switch and Stop rules. Both boards, #2 (UBX) then #1 (OBX), at code `e355fdc`, 12:18–12:30, with no power cycle. Conditions: [conditions.json](conditions.json). Rows: [results/verdicts.tsv](results/verdicts.tsv). Raw logs: [raw.txt](raw.txt).

| Test | #2 UBX | #1 OBX |
|---|---|---|
| (c)1 `hw_n47s5_front_after_a_short_burst`: 6 head lengths × 6 rates, booked at the padded end + g | 36/36 PASS | 36/36 PASS |
| N-H1 `hw_n47s5_long_b_after_a_short_head_gap_below_and_at_the_front`, gap g, 16 trials per rate | PASS (6 rates) | PASS (6 rates) |
| `hw_n47s5_a_long_close_at_100_and_200_msps`: held tail + one zero, next burst 3 samples later, 4 trials per rate | PASS | PASS |
| (c)2 `hw_n47s5_raw_front_gaps` | observation only (726 rows) | observation only (726 rows) |
| (d) `hw_n47s5_held_tail_and_stop`: J(e) rows, c+5 after a two-zero close, R5-m5 at c+3+g (12 trials per rate) | PASS (the Session row INCONCLUSIVE, as before) | PASS (same) |
| `hw_n47s5_repeat_switch`, untimed and timed (switch decided at receipt, E2a) | PASS: row applied, 100 → 200 kHz at the boundary, no seam | PASS (same) |
| `hw_b8_stop_end`, 3 trials | PASS (−4000 … +4000 ticks) | PASS (−5400 … +11000 ticks) |

Decisions it fed:
- #81 and #82 closed: their closing conditions (bench.md, "after step F") hold on both boards.
- The +3 after a long close holds at 200 MS/s on both boards (4 trials each). It was the measured minimum on 2026-11-10, so the margin is still zero.
- E2a's switch at receipt and its Stop rules behave on hardware as in the FakeDevice rehearsals.

Not run: (b), the T1 regression of 9 tests, which includes the two whose expectations step F changed (`hw_b8_burst_one_sample_after_a_burst`, B7's retune). Their FakeDevice rehearsals pass.

Observed, not judged: in (c)1 most rows start the tone 30–37 samples after its booked instant (loop delay about 27). Every row at 2.857 MS/s and 10 MS/s, and a few others, start it 8–12 samples after. All are inside the ±50 tolerance; the onset detector's resolution may explain it. Not followed up.
