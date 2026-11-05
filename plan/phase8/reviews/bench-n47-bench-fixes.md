# Bench 2026-11-05 — test fixes made at the bench (review record)

The owner approved changing ignored hardware tests at the bench after FAILs that were test defects, not Module defects (handoff §6's rule "do not fix code at the bench" concerns the Module; the owner chose to fix the tests and rerun so the day's evidence was usable). Bench notes and logs: `~/.cache/ezsdr-bench/2026-11-05/` (outside the repo). Results: [bench-results.md](../bench-results.md) §「2026-11-05」.

## Changes

- `hw_n47_rx_restart_with_new_values` (T2):
  - The origin is read from the restart's registered SampleClock. The test had expected the request, now + 50 ms, which RM-25 moves by the lead after a stop (+5.88 ms).
  - The request is 200 ms ahead on the 1 MS/s lattice, and a booked origin other than the request FAILs.
  - The tone is judged by `n47::restart_tone_verdict` from the new segment's own samples. Before, windows were cut from the whole capture and sat offset by len mod 25.
  - The verdict starts from the tone's onset: the device delivers about 13 exact zeros and about 10 rising samples at each restart. The onset must be at most `RESTART_ONSET_MAX_SAMPLES` = 40 samples after the booked origin.
- `n47::tx_rate_verdict` and `n47::tail_verdict`: when B's onset is off the control burst's delay, the verdict is INCONCLUSIVE and prints the delta; before, it was a FAIL. The measured deltas were +7 to +14 µs. In `tail_verdict` the onset is judged after the gain check. These are owner decisions.
- Unit tests:
  - `a_receive_restart_rises_at_the_new_tone_after_the_device_fill` covers fill then new tone, old tone, a late onset, the bound and no onset.
  - New cases: a late onset with an unchanged gain must FAIL; B late beyond the loop delay is now INCONCLUSIVE.
- Rows: N47B-07 re-anchored (the onset branch now returns INCONCLUSIVE); N47B-42 added (the onset bound).
- A planned row on the old-frequency window count survived its probe. The check is subsumed by `first_window_verdict`, which already fails any window at the old frequency, so the check was removed rather than kept.

## Review

Opus re-check (effort high, read-only), NEEDS CHANGES. All findings adopted:

| Finding | Severity | Outcome |
|---|---|---|
| Booked origin never compared with the request: a timed start that is not honoured could pass | P2 | Fixed |
| The new tone logic was in-probe only, with no unit test or row | P2 | Fixed: pure function, unit cases, row |
| Onset counted in sample positions with a 100-sample bound | P3 | Fixed: ticks, 40 samples |
| No unit case for `tail_verdict`'s new order | P3 | Fixed |
| A 30-line debugging print | P3 | Trimmed to one line: the fill count and the onset |
| Short old-tone bursts (under 5 samples) can hide in the fill | P3 | Recorded only: the device's fill sets this limit |
| Bound the onset delta (beyond tens of µs → FAIL) | P3 suggestion | Not adopted: it touches the owner's decision to report the delta as INCONCLUSIVE; recorded |

Mechanisms: none counted (test defects of a measurement step).
