# admission-per-origin design note (SB-9a withdrawal) — dispositions and counts

The step: the design note the three-strikes rule asked for after #104 (AGENTS §6; `mech:admission-per-origin`). Way of working: Opus drafted the note ([sb9a-note.md](sb9a-note.md), brief [sb9a-brief.md](sb9a-brief.md)), the owner decided D1 and D2, Fable 5.2 reviewed the direction ([sb9a-fable-review.md](sb9a-fable-review.md)); a Sonnet subagent supervised Go DeepSeek ([sb9a-brief-impl.md](sb9a-brief-impl.md), [sb9a-report.md](sb9a-report.md)); Sol 6.2 reviewed the code ([sb9a-sol-code.md](sb9a-sol-code.md)); fix round 1 ([sb9a-brief-fix1.md](sb9a-brief-fix1.md), [sb9a-report-fix1.md](sb9a-report-fix1.md)); Opus re-check ([sb9a-recheck-fix1.md](sb9a-recheck-fix1.md)); gate: `sb9a-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Fable 5.2, design | 0 | 5 | 4 | the value-kind candidate it named was probed in the step and failed: `admission-per-origin`, fifth instance (no issue) |
| Sol 6.2, code | 2 | 4 | 3 | none counted: F1 a check order introduced and caught in the step; F2 the pre-existing `requested` meaning, split out as #114 for the older producers |
| Opus re-check, fix round 1 | 0 | 1 | 3 | none |

Follow-ups filed: #112 (a lone surrogate in a Python request ends the Session; Fable m2), #114 (MA-12, SB-44 and SB-46 at prepare record a Module's value as `requested`).

## Decisions

- D1 (owner, 2026-11-19, 「推奨で」): accept non-ASCII object keys in documents and Session values; OV-15's canonicaliser orders keys by UTF-16 code units (RFC 8785 §3.2.3); SB-9a withdrawn with its 12 door checks, KC-24 step 4 and the seal-failure condition they guarded.
- D2 (owner, 2026-11-19, 「消して」): EA-5's `malformed` error kind and `RunHandleError::Malformed` removed (no producer left after D1); `schemas/server/reply_frame.v1.json` changes structurally (pre-freeze, no versioning).

## Fable 5.2's review (fable-review.md): no major; D1 and D2 stand — dispositions (orchestrator)
- m1 (`pipeline.rs:~1103` needs a non-panicking shape once `Malformed` is gone; pass on the documents `child_refusal` already parsed): adopted.
- m2 (pre-existing: a lone surrogate in a Python request string makes the server exit and ends the Session, `transport.rs:100-106`; client-side fix in `_client.py:79-87`): its own issue, not this step.
- m3 (the rewritten prepare test must use a key the node does not declare, e.g. `test.zeta`, or MA-12 refuses it first): adopted.
- m4 (rows J256, J237 pin behaviour that survives: retarget, not retire): adopted.
- m5 (texts the note omits: design/06:191, 343, 403; RS-15's clause at design/04:172; OV-15's own sentence; the ledger row; three non-Kernel schema files): adopted.
- The value-kind candidate (declared keys outside the node's capabilities and `requires`, and every Executor/Sink report, `prepare.rs:154-181`): probed in the step with a test first; if it fails, it is this mechanism's fifth instance, fixed with the existing `check_value_kind` at those doors (decided by the orchestrator; no user-visible change beyond a refusal the rules already promise — checked against EA-16 in the step).
- The recommended addition, a check that every sealed Manifest reads back through `Manifest::from_json`: adopted as a test-side check (in the suites that seal Manifests), not a runtime `debug_assert!` or a new `CleanupFailure` (the latter would be user-visible; not proposed).

## Sol 6.2 code review (sol-code.md, b38f98cf): FAIL, 2 majors, 4 minors, 3 notes — dispositions (decided by the orchestrator)
- F1 major, adopted: prepare records the SB-6 kind violations and still hands the rejected configuration to registered prepare checks (a check reading the key as a scalar can panic → KC-30 panic reason, SB-6 violation lost). Fix: when the reported-value kind check finds violations, return them before any registered check runs.
- F2 major, adopted: `Violation.requested` holds the Module's reported value, not what the experimenter asked. Fix: `requested` = the Spec's request for that key when there is one (for a coercion, its `requested`), else `None`; the reported value goes in `reason`. Correct the test that pins the wrong record.
- F3 minor, adopted: `run_child` still parses and hashes each admitted document twice; carry the parse results and section hashes forward (keep the refusal order, the raw-hash fallback for an unparseable document, parent time and lease behaviour).
- F4 minor, adopted: the validate probe corrupts `applied[test.grid]` and `coercions[].applied` together; test each corruption alone (each must fail with its own check removed); W9A-3 split if needed.
- F5 minor, adopted: read-back gaps — the V58 suite's direct `finish()` calls and child Manifests from `run_child` go through the read-back check; the replacement `Emit` test uses a non-ASCII map key; one positive assertion that a Unicode-keyed `Violation.requested` survives into a sealed Manifest that reads back.
- F6 minor, adopted: design/03's stage table lists SB-6 at validate and prepare; design/06's KC-24 sentence before the refusal table no longer names step 4 as a source of violations.
- F7–F9 notes: no action.
- Fix round 1 (cb528cb2) deviations (decided by the orchestrator): F2's `requested` for a coercion entry is also the Spec's `Eq` value or absent, not `Coercion.requested` (a Module's report) — accepted. MA-12/SB-44/SB-46 violations at prepare still record the Module's applied value as `requested` — pre-existing, not introduced here: filed as an issue, not fixed in this step. `reactive.rs`'s direct `finish()` left (F5 named V58).
- Re-check of fix round 1 (recheck-fix1.md): F1–F6 CLOSED, no new major. N1 (SB-41 needs SB-6's exception) and N3 (stale SB-9a citation in a test double) fixed by the orchestrator; N2 (the Spec's request worked out in validation.rs and coercion.rs; they must change together) and N4 (no two-fragment pin) recorded, not fixed (notes).
