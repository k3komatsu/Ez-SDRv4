# e2b-results-3: correction (fix round 3, 2026-11-03)

The previous report called the high condition "unsupported". That was a misreading of the trial files.

- `trial-today.txt` and `trial-new.txt` run the candidates in order and stop at the first that passes. The 62.6k and 75.3k blocks/s figures belong to
  **candidate 0 (200 MS/s x 2,000)**; the candidate the verdict runs use is **candidate 2 (50 MS/s x 500)**.
- At candidate 2 both owners **pass the no-update trial**: 99,999.98 blocks/s, zero overflows, zero missing samples, backlog about 0.05 ms
  (`trial-today.txt:6-8`, `trial-new.txt:6-8`). The no-update condition is supported.
- With timed updates the new runtime **FAILs** the high condition (every repetition, 1,136 to 4,094 updates not applied at their instant), so the
  verdict stands as FAIL; the cause is analysed in `../e2b-results-4/README.md`.
- `trial-after.txt` is a third run of the new runtime at candidate 2 on a loaded host: it shows 31 to 69 ms backlog excursions, which is the host noise
  that `../e2b-results-4/` quantifies (the same host also showed such excursions with no updates in fix round 3).
