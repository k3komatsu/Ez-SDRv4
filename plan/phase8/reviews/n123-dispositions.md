# #123 — dispositions (decided by the orchestrator)

Not risky (a refusal of a malformed Module report at prepare). Implementer: Sonnet (production, spec) driving Go DeepSeek (tests, rows), one DeepSeek round, no send-back. Full mutation lists before Sol: 3568/3569 (phase7/G09, known).

## Sol 6.2 code review (`n123-sol-code.md`): PASS, 3 minors — all adopted and fixed
| # | finding | disposition |
|---|---|---|
| 1 | both omission tests used an empty `effective`, so an `is_empty()` check would pass | fixed: one case keeps a valid `test.count` entry |
| 2 | the success control accepted a report with no coercion, against MA-12 | fixed: the control carries the preview's coercion 19.5 → 20 with `effective` 20 |
| 3 | SB-44's diagnostic said "both values" | fixed: the reason carries the previewed value and the reported value or its absence |

Minor-only fixes: no re-review (AGENTS §9). Rows N123-1..3 killed by assertion after the fix.

## Counts
Findings: Sol 0 major / 3 minor (tests 2, spec text 1). Mechanism: none — the omission sibling of #118's false `applied` (a report field trusted without comparison, #114/#115/#118's family); #118 was recorded `mech: none` for the same reason; no third distinct bug of one design mechanism.
