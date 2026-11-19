# Re-check — #115/#116 fix round (55cefb8b, plus 4efbd4b8 docs), read-only

Reviewer: Claude Opus subagent (re-check). No builds, tests or mutations run; the probe and row evidence read from `ds-fix-1.log` (lines 2519–2600) and `rows-fix.log`.

## Own failure scenarios (written before reading sol-code.md / decisions.md)

1. A Session `SetParameter` the Provider snaps across a check's ceiling: `requested` must be the Action's value, the reason the coerced one. → `admission.rs:142-146` captures `asked` before `coerce_provider` (`:147-150`); `session.rs:233-237` overwrites. OK.
2. A scheduled Action (arm batch `pipeline.rs:822`, agenda `stepping.rs:373`) vs a Session Action (`pipeline.rs:1019`) vs a Module Action (`pipeline.rs:1221`): all go through `admit_with`, so all get the override. Schedule items come from `resolve_template` (the Spec's authored value, `event.rs:924`); the agenda holds only untimed `Stop`; nothing re-admits an already-coerced value. Module: no coercion, `asked == proposed`. OK.
3. Several keys in one Action: impossible — `compile` emits at most one `UpdateParameter` per Session action (`session.rs:347`, `:384`); `asked` has 0 or 1 entries. A `TxBurst`/`Stop` gets an empty `asked`, so every registered-check violation records none (consistent with "the Action asked nothing").
4. `asap` record (`ezsdr.action.at`, `session.rs:355-360`): enters as `incoming` coercion, judged only in step 2 (SB-46) with its own `c.requested`; untouched.
5. A check naming a key the Action did not touch → none (pinned by part (b) of the new kc_24 test).
6. Two fragments with the same key: `asked` is keyed by `Key` only and `Violation` carries no fragment, so a violation for fragment B naming key K gets the Action's value for A.K. `RfEnvelopeCheck` itself is per-fragment (`radio/src/lib.rs:309-312`, `proposed.get(&fragment)`), so here the Kernel is *less* precise than the check it overrides. See N1.
7. validate / prepare: neither constructs an `Admitter` (`session.rs` doc on `Admitter`; only callers of `admit` are three tests, `run_session.rs:966`, `spec_binding.rs:939`, `:4853`). Unchanged.
8. SB-46 violations: built in step 2 after the overwrite loop, from `c.requested` (`coerce_provider` sets it to `requested_value`, the pre-coercion value, `admission.rs:224`). Untouched.
9. A check that relies on its own `requested` at runtime: `TestLimitsCheck::with_non_ascii_requested` (`doubles.rs:209`) is used only at validate (`coordinator.rs:3380-3398`); not affected.
10. KC-24 text vs steps 10/11 rows. See N2.

## Findings of Sol (sol-code.md)

- **F1 (major) — CLOSED.** `admit_with` keeps the Action's value before step 10 (`crates/ezsdr-kernel/src/coordinator/admission.rs:141-146`) and passes it to `Admitter::admit_asked` (`:160`), which overwrites every registered check's `requested` by key, none otherwise (`crates/ezsdr-kernel/src/session.rs:231-237`); the check still judges the coerced `proposed` (`admission.rs:155-158`). Sol's exact sequence (x3xx Mock, `max_gain_db = 2.95`, ask 2.9, snapped to 3.0) is pinned through Session submission by `v58_16_a_runtime_refusal_records_what_the_action_asked_not_the_coerced_value` (`crates/ezsdr-acceptance/tests/v58.rs:587-618`: `requested == 2.9`, reason contains `gain 3 dB exceeds 2.95`, configuration unchanged). KC-24 (`design/06-kernel-coordinator.md:298,301`), SB-30 (`design/03-spec-and-binding.md:284-285`, the method name fixed in 4efbd4b8) and RM-19 (`design/07-radio-model.md:208`) state it.
- **F2 (minor) — CLOSED.** N114-6 is back (`plan/maintenance/tools/mutations.json`, row N114-6) with filter `sb_46_a_rejected_coercion_of_a_key_with_no_exact_request_records_none_at_validate` (`crates/ezsdr-kernel/tests/spec_binding.rs:1987-2008`): `Present(test.flag)`, the stray double `false → true`, policy `reject`; asserts `requested == None` and no SB-44. Killed (`rows-fix.log`).
- **F3 (minor) — CLOSED.** `spec_binding.rs:2055-2058` and `:3081-3083` now say `coerce` is called (SB-7) and returns no coercion; RM-19's Test line (`design/07-radio-model.md:215`) credits the reason only to the prepare test, which asserts it (`crates/ezsdr-radio/tests/radio_model.rs:313,319,325`).

## Do the new tests fail on their named assertion under the smallest wrong change?

| Wrong change (one line) | Test, assertion that fails | Evidence |
|---|---|---|
| delete the overwrite line (`session.rs:234`) — N115F-1 | kc_24 test (a): `requested == 19.5` gets `20.0` (`TestLimitsCheck` records the merged value, `doubles.rs:209-210`), `coordinator.rs:4024` | probe `ds-fix-1.log:2580`, row killed |
| pass `None` instead of `Some(&asked)` (`admission.rs:160`) — N115F-2 / N115F-4 | kc_24 (a) as above; v58 `requested == 2.9` gets `3.0` (`v58.rs:614`) | probes `:2581`, `:2583`, rows killed |
| keep the check's value for an un-asked key (`.or(violation.requested.take())`) — N115F-3 | kc_24 test (b): `requested == None` gets `Some(1.0)` (`coordinator.rs:4041`) | probe `:2582`, row killed |
| build `asked` after `coerce_provider` (move `admission.rs:142-145` below `:148`; no row) | kc_24 (a) 20.0 vs 19.5 and v58 3.0 vs 2.9 | by reading: `asked` would hold the coerced value |
| N114-6: `requested: Some(c.requested.clone())` at validate | `spec_binding.rs:2006` `None` vs `Some(Bool(false))` | probe `:2584`, row killed |

All five fail on the named `requested` assertion, not on an earlier one (the earlier asserts — refusal, count, check, key — do not depend on `requested`).

## New findings

- **N1 — note.** The runtime overwrite is keyed by `Key` only (`crates/ezsdr-kernel/src/session.rs:234`), while `RfEnvelopeCheck` already keys its runtime `requested` by fragment (`crates/ezsdr-radio/src/lib.rs:309-312`). Sequence: a Runtime-stage check flags fragment B's existing `K` while a Session updates A's `K` to v → B's violation (reason names B) records `requested = v`. Not reachable with the in-tree checks: `RfEnvelopeCheck` also runs at validate and prepare, so B's value was already judged; it needs a Runtime-only check (only test doubles have one). The orchestrator accepted this in `decisions.md`; I agree it is not a major. Recorded so the Phase 8-end review knows the rule cannot be made per-fragment without a fragment on `Violation`.
- **N2 — note (pre-existing, not introduced).** KC-24 step 10's table row records, for "the rejected key", "the requested value" — the Action's value for a *different* key (`design/06-kernel-coordinator.md:324`; code `admission.rs:215`, `coercion_violation(fragment, &rejected.key, &requested_value, …)`). That is exactly what the new step-12 sentence forbids ("none for another key"). One KC-24 now carries two meanings of `requested` for a violation naming another key. Candidate follow-up, not part of this round.
- **N3 — note (maintainability, leftover/duplication).** Two places now decide the runtime `requested` of an RF violation: `RfEnvelopeCheck`'s `CheckStage::Runtime` arm (`crates/ezsdr-radio/src/lib.rs:310`) and the Kernel overwrite; through the coordinator the check's arm is always overwritten, so only the direct-call test `rm_19_a_runtime_violation_records_only_what_the_action_asked` and its N116 rows observe it. Likewise `Admitter::admit` (`session.rs:209-217`, `asked: None`) has no production caller left — only `run_session.rs:966`, `spec_binding.rs:939`, `:4853`. Neither is wrong; both could go at the Phase 8-end review.

Nothing else introduced: validate/prepare untouched, SB-46 untouched, workspace reported green (3063 passed, `ds-fix-1.log:2600`; not rerun by me).

**Verdict: F1 CLOSED, F2 CLOSED, F3 CLOSED; no new major or minor; three notes.**
