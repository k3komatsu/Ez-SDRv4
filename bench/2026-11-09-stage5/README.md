# 2026-11-09: the bench after note 47 stage 5 (and the afternoon after stage A)

Conditions: [conditions.json](conditions.json). Results (made by `plan/phase8/tools/bench_extract.py`): [results/verdicts.tsv](results/verdicts.tsv), [results/cells.csv](results/cells.csv). Raw logs: [raw.txt](raw.txt).

- Morning (134b273): (b), (f), (g), (h), (i) passed on both boards; (c)1 failed (#81), (d) R5-m5 failed (#82), (e) failed through a test defect (fixed in stage A).
- Afternoon: the UHD-only reproduction of #81 at 1 and 2 MS/s, 2–4 trials per cell, on #2 only; it set step B's + 10, which the 2026-11-10 sweep showed was a 1 MS/s value ([../2026-11-10-front-sweep/README.md](../2026-11-10-front-sweep/README.md)).
- Prose: [plan/phase8/bench-results.md](../../plan/phase8/bench-results.md) "2026-11-09" and "2026-11-09（午後）".
