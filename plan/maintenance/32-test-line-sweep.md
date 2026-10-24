# 32 — Test-line sweep (2026-10-15/16)

| Field | Value |
|---|---|
| Status | Done for every accepted spec (01–11, 14, 16, 18) at base `0fa2001`; the fixes are one commit. A1's tests landed on 2026-10-16 (rows T01–T33); A2 and B go to Phase 8's plan (owner, 2026-10-16). A2 done on 2026-10-25 (rows T34–T45; [phase 8 reviews/a2-opus](../phase8/reviews/a2-opus.md)): five conditions were already tested, nine got tests. B waits for Phase 8 stage 4. |
| Why | Every Sol review of DA-3 step 2 (specs 07, 09, 18) found Test lines or §6 rows that claimed more than their tests assert. No one had checked that across all specs. |
| Tool | [`tools/sweep/`](tools/sweep/sweep.py): `sweep.py` and its two briefs. Run it again before the freeze review ("全規則の確認", handoff §2). |

## Method

1. **Sweep.** DeepSeek V4.1 Flash, read-only, one fresh run per 15 rules (39 runs, 8 in parallel, on a detached worktree of `0fa2001`). Each run reads every rule's Test line and §6 row, opens each named test and its helpers, and returns JSON: `overclaim`, `silent_gap` (a condition no named test asserts and no `none named` lists), `stale_none_named`, `wrong_test`, each with path:line. `sweep.py check` confirms mechanically that every rule was answered, cited tests exist and evidence lines are in range. Result: 293 rules clean, 228 findings.
2. **Adversarial check.** A fresh DeepSeek run per chunk tried to refute each finding (an assertion in a helper, an unnamed test). 14 were refuted, 214 stood (167 confirmed, 43 partly right, 4 not checked).
3. **Spot check.** Haiku 5.5 checked 41 confirmed findings, up to 3 per spec: 37 were right. All 4 misses had the same shape: the condition was asserted by an existing test that the Test line did not name. So the fix brief makes "name the existing test" the first remedy, and "none named" the last.
4. **Fixes.** DeepSeek, one spec at a time, edited only Test lines, §6 rows and one Changes row per spec. Each finding was re-read against the tests first. Then `rules.py --base 0fa2001`.
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

### Triage (2026-10-16)

DeepSeek read the code behind every `none named` condition the sweep added: 91 rules, 130 conditions. It sorted each into three classes. The full table, with each condition's reason, code location and test sketch, is [`32-triage.json`](32-triage.json).

- **A — needs a test (44).** A wrong implementation could break the condition without any other test noticing, and a small test without hardware can pin it.
  - **A1, Kernel, Sink, channel, Executor and Python (30):** TM-16c, TM-17a, TM-17b, SC-10, SC-20, SC-25, SC-31, SB-13, SB-30, SB-44, RS-24, RS-36, RS-39, KC-28, KC-30, KC-42, HD-8, HD-10, HD-16, CH-9, NX-3, EA-17. Done on 2026-10-16 (owner's decision): one test per condition, mutation rows T01–T33 (T31–T33 added after review), reviewed by Sol twice, and RS-36 now states that `abort` wins over `stop` (implemented and tested before, stated nowhere).
  - **A2, the radio Providers (14):** RM-11, RM-14, RM-18, MR-19, MR-25, MR-29, UR-13, UR-17, UR-18. Phase 8 (owner's decision). Most of them concern overflow and restart reporting, which audit D2 and #51 will change.
- **B — Phase 8, needs the device (4):**
  - SC-25: what the hardware radiated during a gap.
  - RM-23: the receive path delay.
  - UR-3: the per-handle last error.
  - UR-3: `sc16` on the wire.
- **C — stays `none named` (82).** Unreachable by design: a refusal the Kernel already prevents, such as spec 14's missing descriptor or UR-12's source that is not a node. Also forced by types, a process statement, or not worth the cost of a test, such as KC-36's `Release`, which nothing reads.

