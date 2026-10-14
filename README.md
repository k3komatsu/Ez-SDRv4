# Ez-SDR v4

Ez-SDR v4 is a clean-sheet Rust rewrite of Ez-SDR — an SDR experiment runtime built around a small experiment microkernel (the Core tier) and replaceable Modules, with software simulation treated as a first-class execution target equal to physical SDR hardware. Phases 1–7's accepted specifications are in `design/`; the Kernel, Radio Model, Simulation Engine, MockRadio, SimulationChannel, native Executor and server are implemented in `crates/` and `schemas/`, and the Python client is in `python/`. The v3 line lives in a separate repository.

- Project site: <https://k3komatsu.github.io/Ez-SDRv4/>
- [AGENTS.md](AGENTS.md) — the rules for working in this repository.
- [Ez-SDR_v4_ARCHITECTURE_VISION.md](Ez-SDR_v4_ARCHITECTURE_VISION.md) — the design; the accepted specs live under `design/`.
- [handoff.md](handoff.md) — the current state and the next step.
- [k3komatsu/Ez-SDR](https://github.com/k3komatsu/Ez-SDR) — the v3 implementation (behavioural compatibility only, not a template).

Tests:

    cargo test --workspace --no-fail-fast
    PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=~/.cache/cargo-target/Ez-SDRv4/debug/ezsdr-server ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests
