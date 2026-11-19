# #115 and #116 — dispositions and counts

#115: SB-44 refuses a reported coercion whose `requested` is not the experimenter's exact request. #116: the RF-envelope check's `requested` is the Spec's value at validate, nothing at prepare, and — after Sol's F1 — the Kernel sets a registered check's runtime `requested` to the Action's value before coercion. Way of working: a Sonnet subagent supervised Go DeepSeek; Sol 6.2 reviewed the code ([n115-sol-code.md](n115-sol-code.md)); Opus and GPT-6.1 Terra re-checked the fix round side by side ([n115-recheck.md](n115-recheck.md), [n115-terra-recheck.md](n115-terra-recheck.md)). Brief: [n115-brief.md](n115-brief.md); reports: [n115-report.md](n115-report.md). Gate: `n115-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Sol 6.2, code | 1 | 2 | 1 | none counted (F1: the runtime value read after the Provider's coercion) |
| Opus re-check | 0 | 0 | 3 | none |
| Terra re-check | 0 | 1 | 0 | none |

Follow-up filed: #118 (a prepare coercion's false `applied` under `accept`).

## Decisions

- #115: SB-44 refuses a reported coercion whose `requested` is not the experimenter's exact request (Spec `Eq`, a Sink's `params`), at validate and prepare, under every policy; a key with no exact request is not checked. Fires only on a Module bug; no in-tree Provider or Sink trips it (workspace green). EA-16: an ordinary `ezsdr.coercion` violation, no text change.
- #116: the RF-envelope check records the Spec's value at validate, the Action's at runtime, and nothing at prepare (a registered check does not see the Spec at prepare, SB-30); the reported value is in the reason. Adding the Spec to SB-30's inputs for this was rejected: MA-12's violation for the same key at prepare already records the Spec's value.
- Row N114-6 deleted (its mutant is equivalent once SB-44 refuses a lying Module).

## Sol 6.2 code review (sol-code.md): FAIL, 1 major, 2 minors, 1 note — dispositions (orchestrator)
- F1 major adopted: at runtime the RF check's `requested` is the coerced `proposed` value (2.9 dB asked, 3.0 dB recorded). Fix without changing the registered-check interface: at runtime admission the Kernel sets each registered check's violation `requested` from the Action — the Action's original value for the key it names, else none — and the check keeps judging the coerced configuration. KC-24 (and SB-30 if it states what a check's `requested` holds at runtime) says so. A test through Session submission (the Mock x3xx gain case Sol gives) failing on the current code at `requested`, and a row.
- F2 minor adopted: N114-6 retargeted to the reachable non-Eq case (`Present(test.flag)`, the double reporting false → true, policy reject: SB-46 records `requested = None`).
- F3 minor adopted: the SB-30 test comment and RM-19's Test line state what each test establishes.
- F4 note: the false `applied` in a prepare coercion under `accept` (pre-existing) is filed as its own issue; the prepare-only stray double violation left as is.
- Fix round (F1–F3): the runtime override of a registered check's `requested` is by key only (another fragment's violation naming the Action's key would also get the Action's value) — accepted by the orchestrator: another fragment's value for that key was admitted earlier, so such a violation is not expected; the second wrong comment DeepSeek found (`sb_44_a_provider_may_not_exempt_a_key_by_declaring_a_coercion`) fixed by the orchestrator.
- Re-check of the fix round, Opus (recheck.md) and Terra (terra-recheck.md) side by side: F1–F3 CLOSED by both. Terra only: SB-30 still named `Admitter::admit` (minor, fixed by the orchestrator). Opus only: notes — N1 key-only override (already accepted); N2 KC-24 step 10 records the Action's value under the rejected key, against step 12's new "none for another key" (pre-existing, = Sol's #114 F5 note; left with the KC-24 table as it states); N3 the RF check's runtime `requested` branch is now always overwritten and `Admitter::admit` is called only from tests — to the Phase 8-end review (handoff §2's list). Time: Terra 50 s, 76 k tokens, 2% of the 5 h window; Opus 3.6 min.
- mech (orchestrator): #115 and #116 "mech: none" — the same field-meaning mix-up as #114, at the two producers it did not cover.
