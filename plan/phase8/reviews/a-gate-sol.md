# Proposal A #4/#8 — gate command and OpenCode permissions: review record (2026-10-31)

Step: `plan/maintenance/tools/gate.py`, `known-survivors.txt`, project `opencode.json`, shared mutation-result parser in `remote_mutate.py`. Not risky (tooling): no test plan. Implementer Haiku 5.5 (`anthropic/claude-haiku-5-5#high`), no self-review round (brief). Brief and fix brief: [a-gate-brief.md](a-gate-brief.md). Gate manifest of the final tree (worktree, uncommitted files included in its identity): [a-gate-gate.json](a-gate-gate.json) — PASS, workspace 2163, uhd 593, Python 57+57, mutation 2006/2007 (phase7/G09 known).

## Findings and dispositions

| # | Source | Sev. | Finding | Disposition |
|---|---|---|---|---|
| 1 | Sol | P2 | A known survivor also excused a missing row, COMPILE ERROR or NOT FOUND | Fixed: exempt only an observed SURVIVED; every selected row must appear |
| 2 | Sol | P2 | A run without the full mutation (no `--mutation`, or `--ids`) got PASS | Fixed: such runs are PARTIAL; PASS only for the full gate |
| 3 | Sol | P2 | Docker steps shared one Cargo target volume across checkouts (mtime reuse of another tree's artifacts) | Fixed: one volume per checkout, recorded in the manifest |
| 4 | Sol | P2 | An unrelated flaky failure during a mutant's test run counts as a kill | Rejected for this step: pre-existing runner behaviour (`mutate.py`); stays a known limit of the mutation evidence |
| 5 | Sol | P2 | An interrupted rerun left the previous PASS manifest | Fixed: RUNNING written atomically at start, FAIL on any exception, atomic final write |
| 6 | Sol | P3 | Remote mutation images not recorded | Fixed: per-host image id in the manifest |
| 7 | Sol | note | Mutation outcomes parsed twice (gate, `remote_mutate.py`); `killed, but recorded as equivalent` counted as killed | Fixed: one shared parser; that outcome is not a pass |
| 8 | Sol | note | Permission proof ran on OpenCode v2.1.4 | Re-proved on v2.2.5: `git commit` and local `mutate.py` denied under `--auto`, `remote_mutate.py` allowed. Guardrails, not a sandbox (command variants such as `git -C` are not covered) |

Sol re-review of the fix round: PASS, no new findings. Counts: Sol P2 ×5 (4 fixed, 1 rejected), P3 ×1, notes ×2; mechanisms: none assigned (tooling defects, not a design mechanism in `failure-mechanisms.md`).

Process notes: the implementer twice ended its run with a long command still in the background, which killed it (Luna on the rustfmt rows, Haiku on the gate rerun); the brief checklist in handoff §5 now says to wait in the foreground.
