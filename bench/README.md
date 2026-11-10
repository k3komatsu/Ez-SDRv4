# Bench sessions

One directory per bench session, `YYYY-MM-DD-<topic>/` (AGENTS.md §9, Bench records; owner 2026-11-10):

- `conditions.json`: commit, boards (serial, FPGA, daughterboards), attenuation and wiring, reference, UHD image, RF envelope, rates and grids.
- `results/`: machine-readable rows made from the raw logs by `plan/phase8/tools/bench_extract.py`, never typed by hand: `verdicts.tsv` (hardware-test VERDICT lines), `cells.csv` (`tx_repro` sweep cells), `measurements/` (measurement records without their observations). A session whose logs have none of these keeps only the header row; its raw logs hold the rest.
- `README.md`: the human summary and the decisions it fed.
- `raw.txt`: the raw logs' archive in a release of the private repository `k3komatsu/Ez-SDRv4-bench-raw`, with its sha256 and how to fetch it.

Procedures stay in `plan/phaseN/bench.md`; conclusions and the index of sessions in `plan/phaseN/bench-results.md`. The sessions from 2026-10-20 to 2026-11-06 were backfilled on 2026-11-10 (their `conditions.json` comes from the prose record). The Phase 7 bench (2026-09-28 … 2026-10-01) ran on the development machine used before 2026-10-16; its raw logs were not found on this PC or the NAS, so it is recorded only in `plan/phase7/bench-results.md`.
