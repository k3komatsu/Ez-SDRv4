# Bench sessions

One directory per bench session, `YYYY-MM-DD-<topic>/` (AGENTS.md §9, Bench records; owner 2026-11-10):

- `conditions.json`: commit, boards (serial, FPGA, daughterboards), attenuation and wiring, reference, UHD image, RF envelope, rates and grids.
- `results/`: machine-readable rows made from the raw logs by `plan/phase8/tools/bench_extract.py` (hardware-test VERDICT lines; `tx_repro` sweep cells), never typed by hand.
- `README.md`: the human summary and the decisions it fed.
- `raw.txt`: the raw logs' archive in a release of the private repository `k3komatsu/Ez-SDRv4-bench-raw`, with its sha256 and how to fetch it.

Procedures stay in `plan/phaseN/bench.md`; conclusions and the index of sessions in `plan/phaseN/bench-results.md`. Sessions before 2026-11-09 are recorded only in those files; their raw logs (2026-10-27 … 2026-11-06) are still on the development PC under `~/.cache/ezsdr-bench/`.
