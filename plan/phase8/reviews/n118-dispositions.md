# #118 — dispositions and counts

#118: under `accept`, a Provider whose `coerce` previewed `19.5 → 20` could report at prepare `effective = 20` with a coercion claiming `19.5 → 40`, and the false `applied` stayed in the successful prepare report and the Manifest. SB-44a now refuses a prepare coercion whose `applied` is not the validate preview's `applied` for that resource and key, or, with no preview, the report's own `effective` value; with neither it is refused as a malformed report. Within `collect_prepare` one bad key gets one violation: the first of its own checks that refuses a (fragment, key) wins and every later one skips the key. Risk: not risky (a refusal of a malformed Module report). Way of working: a Sonnet subagent implemented (driving Go DeepSeek in the first round, directly afterwards); Sol 6.2 reviewed the code ([n118-sol-code.md](n118-sol-code.md)); GPT-6.1 Terra re-checked each fix round alone ([n118-terra-recheck.md](n118-terra-recheck.md)). Briefs: [n118-brief.md](n118-brief.md). Gate: `n118-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Orchestrator, first diff | 0 | 1 | 0 | none (the violation's check named `ezsdr.effective`; SB-44's other refusals use `ezsdr.coercion`) |
| Sol 6.2, code | 2 | 2 | 0 | none counted |
| Terra re-check 1 | 2 | 1 | 0 | none counted |
| Terra re-check 2 | 3 | 1 | 0 | none counted |
| Terra re-check 3 | 1 | 0 | 0 | none (a mutation row deleted as unreachable) |

Follow-up filed: #123 (a prepare report that omits a previewed key from `effective` passes MA-12).

## Decisions (orchestrator)

- The check lives in `collect_prepare`'s coercion loop, next to SB-44's `requested` check (#115), with the shared lookup `preview_for(admission, resource, key)` that MA-12's re-match also uses. Both comparisons are kept: with a preview the preview decides, without one the report's own `effective`.
- The refusal is an ordinary `ezsdr.coercion` violation (as SB-44's others); it fires only on a lying Module; no in-tree Provider, Sink or fixture trips it (workspace green). EA-16: no text change.
- One violation per (fragment, key) among `collect_prepare`'s own checks: one `refused` set, filled in the order SB-6 kind, SB-44 stray, SB-44 false `requested`, SB-44a, SB-46 policy, MA-12 narrowing, read by every later check including MA-12's re-match. Registered checks (`checks.run`, e.g. the RF envelope) keep reading the unfiltered `effective` and may add their own violation: filtering a safety check's input is not worth one fewer message on a refused Run (Terra re-check 2, finding 3: text narrowed instead).
- SB-6 no longer returns early: a kind refusal skips only the registered checks (which may read a value as its declared kind); MA-12 still runs and skips the refused keys, so another fragment's mismatch still gets its violation.
- Stray coercions: every stray key is reported, one each (was: the first only).

## Sol 6.2 code review: FAIL, 2 majors, 2 minors — dispositions
- F1 major (one bad key, two violations) adopted: precedence in one place, stated in SB-44a.
- F2 major (a coercion with no comparison source passes) adopted: refused as a malformed report; the Provider search found every in-tree producer puts a coerced key in `effective`.
- F3 minor (the SB-6 test sidestepped a duplicate) adopted: its original report restored.
- F4 minor (lookup scope weakly pinned) adopted: one helper shared with MA-12; a two-resource test.

## Terra re-checks — dispositions
- Re-check 1: major 1 (a report that omits a previewed key from `effective`) rejected for #118 and filed as #123 — an omission, MA-12's re-match, outside a false `applied`; to 8b under the Phase 8 issue cut-off. Majors 2 (stray + SB-46, SB-6 + SB-46 duplicates) and minor 3 (SB-44a deferring to an MA-12 check an early return may skip) adopted with one mechanism, the `refused` set. Its note on N118-1 adopted (the fallback value differs from the preview).
- Re-check 2: major 1 (SB-6 twice for a key wrong-kind in `effective` and `applied`) and major 2 (MA-12 narrowing then re-match) adopted; five existing MA-12/SB-46 tests now expect one violation, the re-match test moved to a ranged capability so it still reaches the re-match. Major 3 (registered checks see a refused key) rejected as a code change, adopted as a text fix (above). Minor 4 (two stray keys) adopted.
- Re-check 3: major (row J160 deleted although a `List` key the node does not declare can reach the re-match with a list) adopted: J160 restored and retargeted to a test of that sequence.

## Mutation rows
N118-1…16 (N118-10…16 for the refused set), re-anchored N114-7, N114-9, N115-4, T10, W9A-5, N118-6; J160 restored and retargeted. Every row run in its fix round was killed by assertion; the full lists run at the gate.

## mech
#118 "mech: none" — the same field-meaning mix-up as #114/#115 (a reported coercion's fields trusted without a comparison), at the one field they did not cover; no new mechanism.
