# 2026-11-10: the Module tests after E1, and the front swept 0.1–200 MS/s

Conditions: [conditions.json](conditions.json). Results (made by `plan/phase8/tools/bench_extract.py`, not by hand): [results/verdicts.tsv](results/verdicts.tsv) (hardware-test VERDICT lines), [results/cells.csv](results/cells.csv) (one row per sweep cell: board, rate, case, first send, head length, gap, trials, trials that played). Raw logs: [raw.txt](raw.txt).

- Module tests at f127246 with step B's front (+ 10): (c)1 passed at 1 MS/s and failed at 2 MS/s on both boards; (d) R5-m5 at c + 13, N-H1 and `restart_trial` passed (1 MS/s only).
- The UHD-only sweep (both boards identical): after a short closed head, interpolation 60–400 needs up to about 10 µs, 4–50 needs one sample, 100/200 MS/s need 2/3 samples after any close; at 0.4 MS/s and below a closed burst of 4 samples or fewer underflows itself.
- Owner decisions fed by it: the time-based front g = max(3, ⌈15 µs × rate⌉) and + 3, transmit rates ≥ 1 MS/s (step F, [plan/phase8/reviews/stepF-dispositions.md](../../plan/phase8/reviews/stepF-dispositions.md)).
- Prose and tables: [plan/phase8/bench-results.md](../../plan/phase8/bench-results.md) "2026-11-10".
