# Re-check of fix round 1 (cb528cb2 against b38f98cf) — Claude Opus, read-only

Scope: `git diff b38f98cf cb528cb2`, the code it connects to, sol-code.md, decisions.md, report-fix1.md, rows-fix1.log and rows-fix1b.log. Verification: static reading, plus one targeted run in a `git archive` copy at /tmp/recheck-nsb9a with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`. The run was `cargo test -p ezsdr-kernel --test spec_binding --test coordinator -- sb_6 sb_29 kf_03 ma_12_a_widening kc_24_a_module_emit`, and it passed 12 + 7. `cargo fmt --all --check` is clean. No mutant was applied and `mutate.py` was not run; the kill evidence is the two row logs.

## Verdicts

| Finding | Verdict |
|---|---|
| F1 (major) | CLOSED |
| F2 (major) | CLOSED |
| F3 (minor) | CLOSED |
| F4 (minor) | CLOSED |
| F5 (minor) | CLOSED |
| F6 (minor) | CLOSED (one sentence elsewhere left; see N1) |

### F1: closed
- **Early return.** The return sits at `crates/ezsdr-kernel/src/plan/prepare.rs:156-158`.
  - It comes after the per-fragment loop, which sets `kind_refused` at `prepare.rs:108`.
  - It comes before `checks.run` at `prepare.rs:161` and before MA-12's loops at `prepare.rs:168-252`.
  - The loop does not break on the first wrong-kind value. Every fragment's SB-6 violations are therefore collected, along with that loop's SB-44 and SB-46 violations.
- **Nothing reads the report in between.** The only reads after the kind check are SB-44's key test (`prepare.rs:129-138`) and SB-46's `apply_coercion` (`coercion.rs:32-41`). The latter formats `applied` with `{:?}` and reads no kind.
- **Recording.** `PrepareError::Violations` becomes `fail(Stage::Prepare, "KC-12: {violations:?}")` (`coordinator/pipeline.rs:622-625`). `self.reports` and the configuration are set only on `Ok` (`pipeline.rs:627-630`), so the refused report reaches neither KC-27's configuration nor the Manifest's `prepare` section (`ending.rs:88`).
- **Other stages.** No other stage hands a reported value to a check.
  - Validate checks get the Spec's `Eq` values and output params (`plan/validation.rs:25-41`).
  - The runtime point gets the coordinator configuration, which is set only from an accepted prepare, and the kind check of KC-24 steps 6 and 11 runs before it.
  - At validate, a wrong-kind `applied` entry or coercion is used only by `contains_key` (`matching.rs:141`). It is excluded from `coercions_preview` (`matching.rs:183`).
- **Skipping MA-12 loses no refusal.** The Run is refused by SB-6 on this path either way, and nothing transmits (SB-30). What is lost is only the extra MA-12 and registered-check violations in the same failure's reason. No user-facing promise lists every prepare violation; KC-24's "with every violation" is about runtime compile items (design/06:344). MA-12's list branch keeps its coverage through the `ext.` key case (`spec_binding.rs:2279-2301`), and J159 and J160 are killed (rows-fix1b.log).
- **Neighbouring sequences, by inspection:**
  - One fragment wrong and one valid: both fragments are kind-checked, then the early return fires and no check sees either fragment. Correct, though no test pins it (note only).
  - Only `coercions[].applied` wrong: refused, `requested` is the Spec's `Eq`, and the SB-46 policy (Accept) adds nothing. Pinned by case (c) of `sb_6_a_prepare_kind_refusal_records_the_specs_request_and_not_the_reported_value` (`spec_binding.rs` around 2440).
  - Executor or Sink report: `spec.resources.get` gives `None`, so `requested` is absent and the early return still applies. Pinned by `sb_6_an_executor_prepare_effective_of_the_wrong_kind_is_refused` (`coordinator.rs:420-425`).
- **Tests.** The Run-level test (`coordinator.rs:428`) fails at `reason.contains("SB-6:")` and `!reason.contains("panick")` when the early return is removed. Without it, `contain_all` turns the check's panic into "KC-30: a Module panicked during prepare" (`pipeline.rs:614`). W9A-5 is killed (rows-fix1.log:35).

### F2: closed
- **Producers.** The step adds two kind-violation producers, `matching.rs:55-64` and `prepare.rs:106-116`. Both now use `coercion::exact_request` (`coercion.rs:15-20`). No other step-added violation sets `requested` from a reported value; a grep of `requested:` in the step's src diff shows only these two.
- **`exact_request` by case:**
  - `Eq`: the Spec's scalar.
  - Range, Min, Max, Set or Present: `None`.
  - A key the request does not name, and every Executor or Sink fragment: `None`.
  - Session: the implicit Spec's initial values are `Eq` constraints (`session.rs:463`), so a Session gets its initial value.
  - `ext.` keys: `reported_kind_error` returns `None` (`coercion.rs:8-10`), so no violation is made at all.
- **The coercion deviation is right.** The fix uses the Spec's `Eq` for a coercion entry rather than `Coercion.requested`. `Coercion.requested` is the Module's own report; the schema calls it "What the document asked for.", but nothing checks that it is.
- **Descriptions agree.** `Violation.requested` reads "What was asked." in `binding.rs:285` and in the Violation `$defs` of `schemas/violation.v1.json`, `manifest.v1.json`, `admission_result.v1.json`, `log_entry.v1.json` and `server/reply_frame.v1.json`. SB-6's text (design/03:159) now says the same.
- **Tests and rows.**
  - At validate, the pin is `requested == Some(19.5)` plus a reason containing `twenty`; W9A-6 is killed.
  - At prepare, cases (a), (b) and (c) are pinned; W9A-7 is killed.
- **Pre-existing, not introduced here.** MA-12, SB-44 and SB-46 at prepare still record a Module value as `requested` (`prepare.rs:134,148,193,225,244`). decisions.md files that as an issue.

### F3: closed
- **Single parse and hash.** `run_child` (`pipeline.rs:1068-1099`) calls `child_refusal` once. The admitted arm takes `sections.0.hash` and `sections.1.hash`, which are `SpecSection::of` and `BindingSection::of`, that is `ContentHash::of(parsed)` (`manifest.rs:133-164`). Those are the values the old code computed.
- **Refusal path.** Only the refusal path calls `child_action_as_sent` (`pipeline.rs:1101-1116`), which keeps the parsed-or-raw fallback.
- **Behaviour kept.** `now` is still read before either branch. The refusal order inside `child_refusal` is unchanged. The parent's host clock, checks and lease handling in the admitted arm are moved verbatim. `child_refusal` is pure, so evaluating it before the action changes nothing observable.
- **Rows match their names:**
  - J81 and J82 put the as-sent hash into the admitted arm, killed by `kf_03_a_child_run_is_admitted_logged_run_and_recorded`'s `assert_eq!(entry.action, …)`.
  - W9A-8 and W9A-9 put the as-sent hash on the parsed branch of the refusal helper, killed by `kf_03_rs_25a_refusals`'s recomputed `assert_eq!(entry.action, …)`.
  - None of the mutants can panic, so each kill is an assertion. J81 and J82 being killed shows that `spec_one()` and `profile_one()` hash differently as sent and as parsed. That is what lets W9A-8 and W9A-9 be killed on the parsing refusal cases. All four are killed (rows-fix1b.log).

### F4: closed
- **Doubles.** `wrong_applied_grid` and `wrong_coercion_grid` are independent (`doubles.rs:697-707`).
- **W9A-3 (applied loop off).** With only the applied map wrong, the coercion stays valid and is previewed, so `refused.len() == 1` and `coercions_preview.is_empty()` both fail.
- **W9A-4 (coercions loop off).** With only the coercion wrong, it is previewed with a wrong kind, and the same two assertions fail.
- Both rows are killed.

### F5: closed
- No `.finish()` call is left in `ezsdr-acceptance/tests/v58.rs`; all 9 now go through `finish_checked`.
- Every `run_child` Manifest in the `kf_03_*` tests goes through `assert_reads_back`.
- The `Emit` payload's key is non-ASCII (`coordinator.rs:3851`).
- `sb_29_…` (`coordinator.rs:3381`) pins a Unicode-keyed `requested` in a sealed Manifest that reads back.
- `reactive.rs` is left, as decisions.md accepts.

### F6: closed
- The stage table now lists SB-6 at validate and prepare (design/03:262,264).
- KC-24's sentence now reads "steps 1–3, 5–8, 10 and 11" (design/06:303).
- The sentence N1 names was not one of F6's locations.

## New findings (none major)

- **N1, minor.** SB-41 (design/03:313) still says without qualification that `prepare` "runs SB-30's second point — every registered check … — before it returns" and "also runs MA-12's two obligations". After F1, both are skipped when SB-6 refuses a report.
  - SB-6 states the exception, but SB-41's sentence is now false on that path. The AGENTS §9 checklist asks for every `design/` text the change makes false.
  - Fix: one clause in SB-41, "unless SB-6 refused a report", citing SB-6 rather than restating it.
- **N2, note.** The "Spec's request of a key" mapping (`Eq` to `Value::Scalar`) now exists twice: `plan/validation.rs:26-33` (`eq_values`, SB-30's validate point) and `plan/coercion.rs:15-20` (`exact_request`).
  - They agree today. One difference: the validate point also treats a Sink output's `params` as requested, while `exact_request` returns `None` for every Sink fragment.
  - That is allowed by "the Spec's `Eq` or nothing", but the two definitions of "what was asked" must change together.
- **N3, note.** The `TestLimitsCheck::with_non_ascii_requested` doc comment (`tests/support/doubles.rs:169-170`) cites "(SB-9a)", which this step withdraws. The test that uses it (`coordinator.rs:3381`) explains this correctly. Cite SB-4 instead.
- **N4, note.** Without the early return, `sb_6_a_prepare_kind_refusal_is_returned_before_a_registered_check_runs` (`spec_binding.rs:2462`) fails from the double's `expect("a scalar")` panic, not at its counter assertion; report-fix1 says so too.
  - The Run-level test carries W9A-5 and fails at its named assertions, so this is acceptable.
  - No test pins the two-fragment case of F1 (one wrong, one valid).

Not checked: Python suites; the full workspace and full mutation lists (I relied on report-fix1's counts and the two row logs); hardware.
