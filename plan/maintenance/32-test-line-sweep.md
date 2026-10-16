# 32 — Test-line sweep (2026-10-15/16)

| Field | Value |
|---|---|
| Status | Done for every accepted spec (01–11, 14, 16, 18) at base `023eaa7`; the fixes are one commit. **Open:** which untested conditions get a test (below), an owner decision. |
| Why | Every Sol review of DA-3 step 2 (specs 07, 09, 18) found Test lines or §6 rows that claimed more than their tests assert. No one had checked that across all specs. |
| Tool | [`tools/sweep/`](tools/sweep/sweep.py): `sweep.py` and its two briefs. Run it again before the freeze review ("全規則の確認", handoff §2). |

## Method

1. **Sweep.** DeepSeek V4.1 Flash, read-only, one fresh run per 15 rules (39 runs, 8 in parallel, on a detached worktree of `023eaa7`). Each run reads every rule's Test line and §6 row, opens each named test and its helpers, and returns JSON: `overclaim`, `silent_gap` (a condition no named test asserts and no `none named` lists), `stale_none_named`, `wrong_test`, each with path:line. `sweep.py check` confirms mechanically that every rule was answered, cited tests exist and evidence lines are in range. Result: 293 rules clean, 228 findings.
2. **Adversarial check.** A fresh DeepSeek run per chunk tried to refute each finding (an assertion in a helper, an unnamed test). 14 were refuted, 214 stood (167 confirmed, 43 partly right, 4 not checked).
3. **Spot check.** Haiku 5.5 checked 41 confirmed findings, up to 3 per spec: 37 were right. All 4 misses had the same shape: the condition was asserted by an existing test that the Test line did not name. So the fix brief makes "name the existing test" the first remedy, and "none named" the last.
4. **Fixes.** DeepSeek, one spec at a time, edited only Test lines, §6 rows and one Changes row per spec. Each finding was re-read against the tests first. Then `rules.py --base 023eaa7`.
5. **Review.** Sol 6.1 reviewed in four batches (01–04, 05–08, 09–16, 18): CHANGES_REQUIRED, 23 P1 in all. Most were older overclaims the sweep had missed; a few were new (a check name that no test asserts; a `none named` that a schema-freeze test pins). DeepSeek fixed them, plus sibling rows with the same defect. Sol re-checked all four batches: PASS.

Cost: about $3 of DeepSeek (paid `opencode-go`) and eight Sol runs.

Things learned:
- The free models (`opencode/deepseek-v4.1-flash-free`, `opencode-go/step-5-preview-free`) hit 429 within minutes under parallel load.
- Sol runs started at the same instant all hung at 0 bytes. Start each one only after the previous one is writing.
- A DeepSeek session can end with "Session interrupted: inactivity" and leave an old report's STATUS line behind. Check the log, not only the report.

## Open: conditions no test asserts

The commit adds 112 `none named` clauses to the specs' Test lines. Each one is a condition that no test asserts and that the audit and Sol found worth stating. Examples:

- KC-36's `Release` (no test exercises it).
- spec 14's "no descriptor for a component" refusal (unreachable from the tests).
- UR-12's refusal of a `source` that is not a node.
- RM-8's `Max`/`Range` applied values.
- RM-23's receive path delay.

Which ones get a test, and when, is the owner's decision, item by item. The candidates are the `none named` lines in `git show <this commit> -- design/`.
