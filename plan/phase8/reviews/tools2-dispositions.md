# tools2 — note 52 D2 and D1's trial: dispositions and counts

D2: the mutation image is built on `ezsdr-v4-dev:uhd4.10` (non-root, Rust stable, numpy, rsync, cargo-mutants pinned), rows may carry `features` (row TL2-1 on `uhd.rs` with `["uhd"]`), and each run builds and removes its own image tag. D1's trial: `plan/maintenance/tools/remote/sweep_mutants.py` runs cargo-mutants on sim02/sim03 in containers on its own copy; the trial's numbers and survivor sample are in [tools2-trial.md](tools2-trial.md). Risk: not risky (tooling). Way of working: a Sonnet subagent implemented; GPT-6.1 Terra reviewed and re-checked ([tools2-reviews.md](tools2-reviews.md)). Briefs: [tools2-brief.md](tools2-brief.md). Gate: `tools2-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Terra, review | 3 | 2 | 2 | none (`string-composed-identity` does not fit: the shared image tag was a fixed name, not a composed one) |
| Terra, re-check 1 | 3 | 0 | 0 | none |
| Terra, re-check 2 | 2 | 1 | 0 | none (cleanup that stops at the first failing host; unnamed list/baseline containers; a malformed record outside the FAILED report) — fixed in a last round without further review (tool robustness, orchestrator) |

## Decisions
- Outcome of D1 (owner 2026-11-20 「D1は推奨でok」): no whole-workspace generator sweep (≈14.7 h estimated on sim02+sim03, conditional; over a night); the tool stays for per-crate sweeps of pure-rule crates before the freeze (handoff §4, AGENTS Safety core); the `ezsdr-radio` gaps and the `uhd.rs` row candidates go to 8b (note 52 §4).
- Review dispositions (orchestrator), all adopted: a per-run image tag, then a random run id (Terra 1, re-check 1); a sweep refuses a partial, stale or empty-shard result and exits non-zero (Terra 2, re-check 3); cleanup of dirs, images and containers on every exit path (re-check 2); the trial report keeps the 38 workspace-test timeouts unresolved and states the estimate as conditional with its break-even (Terra 3, 4); return codes of `list` and `baselines` checked, and the sweep's docstring requires `simq` (Terra 5).
