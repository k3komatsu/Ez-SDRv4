# Brief: note-45 experiment E4b — the speed re-measurement

You are the implementer (AGENTS.md §9). Repository `~/local_work/Ez-SDRv4`, branch `main`; you are the only editor of the tree. Never read, list or search `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/` or `holdout-e4/`. Never git commit, push, stash, reset or checkout; never run `mutate.py`/`remote_mutate.py`.

This is measurement tooling (no test plan; owner 2026-10-26). **No self-review** (owner 2026-11-03); Sol reviews the code and the results. Mechanical rules: probes count a compile failure as not killed; the report names the killing test per new row; every run's output is saved and named.

## What it measures
`plan/phase8/45-experiment.md` §11 "Fixed before measurement" and "Criteria", unchanged from E2b (`plan/phase8/reviews/45e2b-results.md`, `45e2b-brief.md` and its fix briefs in `~/.cache/ezsdr-fixes/2026-11-01-n45/brief-e2b*.md`): p99(new) − p99(today) < 100 µs, p99(new) ≤ 300 µs, loss(new) ≤ loss(today), every update accounted, at 1 MS/s × 2,000 and at 50 MS/s × 500 (100,000 blocks/s), spaced and bursty, same update table, same ingress rule, same fail-closed per-repetition verdict, loss counted from the reader. K = 4 (device), D = 64 (per chain) — the defaults since E4a1; no override in the verdict runs. FakeDevice is not changed.

## Tooling changes (small; under `src/brain/speed/` only)
1. The bound diagnostic's per-second samples report K occupancy (`device_load` at the brain's time, end and max) and D occupancy (the chain's resident count, end and max) separately, replacing the single in-flight occupancy; the per-update table keeps first attempt → reserved (held time), apply entry/return, application tick, retirement.
2. Rejections by reason already split K and D (`REASONS`); make sure the verdict JSON reports K hits and D hits per repetition, and the max K and D occupancy per repetition.
3. A per-repetition "coupling check": for every late update (not applied at e), whether a D hit happened at or before its release on its chain in that repetition, and whether the R backlog exceeded 1 ms then. Report counts: late updates, late with a preceding D hit, late with a preceding K hit, late with neither.
Unit tests for any new pure computation (the coupling check), with rows if a behaviour is added (next free E4 id).

## Runs (on one sim host through `plan/maintenance/tools/remote/rdev.py`, one at a time, `uptime` recorded before each; copy `~/.cache/ezsdr-fixes/2026-11-01-n45/e2b-results-3/run-all.sh` and E2b-results-4's `run-bound.sh` into `~/.cache/ezsdr-fixes/2026-11-01-n45/e4b-results/` and adapt)
- the two trials (today, new; 50 s);
- the four verdicts (1 MS/s and 50 MS/s × 500; spaced, bursty; 10,000 updates; the repetitions E2b used);
- the two diags and the fault run;
- the bound diagnostic at the defaults (K = 4, D = 64) for spaced and bursty, ABBA as E2b did (order "default, default" is fine: no second bound value to compare).
If a host's load differs much from E2b's (16–18 on sim03), say so; do not rerun to chase a better number.

## Results
Write `~/.cache/ezsdr-fixes/2026-11-01-n45/e4b-results/README.md` with the same table as `45e2b-results.md` (p50, p99, p99.9, failures, loss, verdict per condition and owner), the K/D hit counts and max occupancies, the coupling check, and the verdict per §11 (including "D hits precede the late updates → failed").

## Checks
Brain suite, clippy `-D warnings`, `cargo fmt --all`, chmod (`git ls-files -m -o --exclude-standard | xargs chmod go+r`), `rules.py --base HEAD`.

## Report (at most 20 lines)
The table in short, the K/D and coupling counts, the verdict per condition, host and load, the tooling diff size, rows, anything wrong in this brief.
