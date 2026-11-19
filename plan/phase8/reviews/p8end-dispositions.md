# Phase 8-end review — dispositions and counts

The Phase 8-end review of code and test architecture (handoff §2): an Opus draft, measured at main `4679abc1` (scripts in [52-measure](../52-measure/)), owned by the orchestrator and decided by the owner item by item on 2026-11-20 (D1–D7, [note 52](../52-phase8-end-review.md) §4). The two code items it decided "now" landed with it: kc_36's two TTL tests on the manual HostClock (D3, gate trust) and the removal of the stat `tx_close_past_record_front` (D7). Way of working: a Sonnet subagent implemented both directly (no DeepSeek; small); the step is not risky by AGENTS §9 step 1 (a test's clock and a stat nobody read), so GPT-6.1 Terra alone reviewed it (D4's rule). Briefs [p8end-briefs.md](p8end-briefs.md), reviews [p8end-reviews.md](p8end-reviews.md). Gate: `p8end-gate.json` (beside this file).

User-visible: the Manifest `stats` key `tx_close_past_record_front` is gone (owner D7); AGENTS.md: the generator rule (safety core, §7) and the review rules (§9, D4).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Terra, 6cf43263 | 1 | 2 | 0 | none |
| Terra, re-check 7d32c187 | 1 | 0 | 0 | none |

## Decisions (orchestrator unless marked)

- Owner 2026-11-20: D1–D7 as recorded in note 52 §4. Owner 2026-11-20 「うん」: the D1 trial and D2 image move before Gate X, into the wait for the 2026-11-27 bench (tool-only work).
- Terra F1 (major, adopted): `52-measure/commands.sh` hard-coded `4679abc1` as "now", so the 8b-end re-measure (D3) would compare Phase 8 with itself. Fixed: the end commit is the second argument (default HEAD); `4679abc1` stays as Phase 8's end.
- Terra F2 (minor, adopted): the D1 trial's timing differed between AGENTS §7, plan 50 and note 52 — aligned on "before 8b" with the owner's move recorded.
- Terra F3 (minor, adopted): note 49's two passages on the stat marked as removed (D7).
- Terra re-check F1 (major, adopted): `steps.py` put every commit from 11-07 on into the late-Phase-8 cohort, so 8b's churn would mix into it. Fixed: `EZSDR_8B_START` (the day after Gate X) gives 8b its own cohort. No further re-check: a measurement script outside the gate, the fix is one date comparison (orchestrator; D4 allows one re-check).
- Implementer's note: the C07 row's killer is not named in the remote log, so whether the two kc_36 tests are its killers is not confirmed from the log; their assertions are unchanged and the row is killed.
