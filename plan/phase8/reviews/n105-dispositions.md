# #105 / #104 — review record and dispositions (2026-11-18)

Small Kernel drifts from the 2026-11-15 rule sweep (#105: `EventSource::Kernel {}` / `Unforeseen {}` so `deny_unknown_fields` holds; `WITHDRAWN` is the 12 Kernel-prefix withdrawn ids; MA-20 and MA-29 doc comments and their schemas; OV-23b counts public items only, 113 of 293) and SB-9a's entry checks (#104: a Provider's `PrepareReport` at prepare, a `CoerceReport`'s coercions at validate — strays included — and at admission, and a registered admission check's `Violation.requested` recorded as absent).

Not risky (no queue, deadline, identity, lifecycle, FFI or fake boundary): no test plan. Lane 2 worktree, branch `work/n105` from `ce631495`, rebased onto main and squashed. Implementer: Go DeepSeek V4.1 Flash (`#max`), self-review one round by Haiku 5.5 ([n105-briefs.md](n105-briefs.md)).

## Findings by source and severity

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Self-review (Haiku 5.5) | 3 | 2 | — | — (a missed SB-9a door at admission; OV-23b's text vs the issue; four rows whose `new` no longer compiled) |
| Sol 6.1, code ([n105-sol-code.md](n105-sol-code.md)), FAIL | 1 | 1 | — | — (a stray coercion transcribed before the SB-9a check; a test that iterated the list it guarded) |
| Opus re-check (fix round 1), PASS | 0 | 0 | 3 | — |

No `mech:` label fits: the misses are entry points of one rule not yet enumerated, found by the reviews that enumerate them.

## Dispositions (decided by the orchestrator)

- OV-23b: the rule text (8d876903) said the `NEW:` count includes the `FEATURE:testing` line; #105 item 5 said it should not. The count measures public items, so the text now says "excludes" and the test asserts 113 of 293.
- Self-review findings 1 (the admission door), 3 (rows J164–J167), 6, 9 fixed; 2 settled by the OV-23b decision.
- Sol 1 (major) fixed: the SB-9a check runs on every returned coercion before SB-44's stray split (row N105-4). Sol 2 (minor) fixed: the five new withdrawn ids asserted literally. Sol's inventory, a pre-existing open door: a registered check's `Violation.requested` with a non-ASCII key is recorded as absent and the refusal stands (row N105-5); SB-9a says so (re-check note 1). Sol's other inventory rows (control-event payloads, sections at cleanup, `ModuleError.detail`, artifacts) were found already checked or unreachable.
- Re-check notes 2–3 (the stray test stops at `validate`; two non-ASCII coercions give two identical violations; no row on the SB-44 split) left as they are: pre-existing or sufficient.
- Implementer's choice kept: `ezsdr.coercion` as the namespace of the coercion refusals.

## Gate

`n105-gate.json` (beside this file).
