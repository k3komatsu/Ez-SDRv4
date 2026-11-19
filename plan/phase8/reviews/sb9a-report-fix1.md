The fix round is done. F1–F6 are fixed. Nothing is committed, staged, stashed or pushed. The tree is `/home/komatsu/local_work/Ez-SDRv4-nsb9a` (`work/nsb9a`, HEAD `b38f98cf`), and I touched no other worktree.

## Diff against `b38f98cf`
`git diff --stat` shows 13 files, +430/−75:

- **Production:** `pipeline.rs`, `plan/coercion.rs`, `plan/matching.rs`, `plan/prepare.rs`.
- **Kernel tests:** `coordinator.rs`, `spec_binding.rs`, `support/doubles.rs`, `support/mod.rs`.
- **Acceptance tests:** `ezsdr-acceptance/tests/support/mod.rs`, `v58.rs`.
- **Design:** `design/03-spec-and-binding.md`, `design/06-kernel-coordinator.md`.
- **Rows:** `plan/maintenance/tools/mutations.json`.

## Findings
- **F1 (prepare handed a refused report to registered checks).** In `collect_prepare`, a `kind_refused` flag is set by `report_kind`. Right after the per-fragment loop, and before `checks.run(...)`, `if kind_refused { return Err(PrepareError::Violations(violations)); }` returns. That return is the only change of order.
  - The SB-44 and SB-46 violations from the same loop travel with the return. The MA-12 loops are skipped on this path.
  - Tests:
    - `sb_6_a_wrong_kind_report_ends_the_run_with_the_sb_6_violation_not_a_check_panic` (Run level) asserts the reason has `SB-6:` and not `panick`, and that the check's call counter is 0.
    - `sb_6_a_prepare_kind_refusal_is_returned_before_a_registered_check_runs` asserts the counter is 0 on the wrong-kind report and 1 on a valid one.
  - Row W9A-5 (`if kind_refused` → `if false && kind_refused`) is killed by the Run-level test.
- **F2 (`requested` held the Module's reported value).** I added `coercion::exact_request(req, key)`. It returns the Spec's `Eq` value for the key as a `Value`, and `None` for a range, a set, or a key the Spec does not name. Both producers use it, and the reported value now goes into `reason` as `{value:?}`.
  - **Other producers.** I checked every `check_value_kind` caller. The runtime door in `admission.rs` (steps 6 and 11) already sets `requested` to the proposed or requested value, and the `validation.rs` callers return `Err(SpecError)`, not a `Violation`. So the two new producers are the only ones that changed.
  - **Tests.**
    - `sb_6_a_coerce_applied_of_the_wrong_kind_is_refused_at_validate` (changed) now asserts `requested == Some(19.5)` and that the reason contains `twenty`.
    - `sb_6_a_prepare_kind_refusal_records_the_specs_request_and_not_the_reported_value` (new) covers three cases: a key the Spec names with `Eq` (`requested` is 20.0), a key it does not name (`None`), and a coercion whose own `requested` of 7.0 is ignored (`requested` is 20.0).
  - **Rows.** W9A-6 (validate) and W9A-7 (prepare) restore `requested: Some(value.clone())`. Both are killed.
- **F3 (`run_child` parsed and hashed an admitted child twice).** `run_child` now matches on `child_refusal` once. The admitted arm takes `spec_hash` and `binding_hash` from `sections.0.hash` and `sections.1.hash`. The old as-sent hashing block moved into `child_action_as_sent`, which runs only on the refusal path. Refusal order, the raw-hash fallback for an unparseable document, and parent time and lease behaviour are unchanged.
  - **Tests.** No new test: `kf_03_rs_25a_refusals` already recomputes the parsed-or-raw hashes per refused case, and `kf_03_a_child_run_is_admitted_logged_run_and_recorded` already pins the admitted hashes.
  - **Rows.** J81 and J82 are retargeted to the admitted arm (as-sent mutants), and W9A-8 and W9A-9 are new on the refusal-path helper, killed by `kf_03_rs_25a_refusals`. J81, J82, W9A-8 and W9A-9 are killed. DeepSeek probed that the raw and parsed hashes differ for the `other_selector` case, so the rows are not equivalent.
- **F4 (validate probe corrupted two fields together).** The `TestProvider` double now has `wrong_applied_grid` and `wrong_coercion_grid`. `with_wrong_grid` sets both; there are two single-sided builders.
  - **Tests.** `sb_6_a_coerce_applied_map_entry_of_the_wrong_kind_is_refused_at_validate` and `sb_6_a_coercion_applied_of_the_wrong_kind_is_refused_at_validate` each assert one SB-6 violation, `requested == Some(19.5)`, not admitted, and an empty preview.
  - **Rows.** W9A-3 is retargeted to the applied-map loop and W9A-4 is new for the coercions loop (`if false && refuse_wrong_kind(..)`). Both are killed.
- **F5 (read-back gaps).**
  - **Acceptance read-back.** `finish_checked` in the acceptance support is now `pub`, and all 9 `RunHandle::finish()` calls in `v58.rs` use it.
  - **Child Manifests.** `assert_reads_back(&Manifest)` was added to the kernel test support, and `FinishChecked` now calls it. It runs on every Manifest `run_child` returns, in the `kf_03_*` tests.
  - **Emit payload.** The payload in `kc_24_a_module_emit_is_dispatched_to_its_target` now has the non-ASCII key `名前`.
  - **Positive pin.** `sb_29_a_registered_checks_violation_requested_with_a_non_ascii_map_key_is_recorded_and_the_manifest_reads_back` restores the `with_non_ascii_requested` double from history. It asserts that `requested` equals the Unicode-keyed map literal and that the sealed Manifest reads back. No row for any F5 item, because it is test infrastructure plus a positive pin.
- **F6 (design text).**
  - **Stage table.** The `validate()` row lists the kind rule on a `CoerceReport`. The `collect_prepare()` row lists SB-6's kind rule, whose refusal returns before any registered check reads the report.
  - **SB-6.** The rule and its Test line now say `requested` is the Spec's `Eq` value or absent, and the reason carries the reported value. The new tests are named, and the Changes row is extended.
  - **KC-24.** The sentence before the refusal table now reads "steps 1–3, 5–8, 10 and 11".

## Check counts
- **fmt:** `cargo fmt --all --check` is clean.
- **clippy:** `--workspace --all-targets -D warnings` is clean.
- **Workspace tests:** 74 binaries, 3046 passed, 0 failed. This includes `schema_freeze`; no schema file changed.
- **Python:** not run, because nothing Python-visible moved. `Violation`'s doc comment is unchanged, so no schema or reply changed.
- **`rules.py --base 7b676bce --renames …/renames.txt`:** 0 problems.
- **Rows:** the first run covered 75 ids; 94 of 99 rows were killed. The five that did not pass were J81, J82, J159, J160 and G09. G09 is the known survivor in `known-survivors.txt`. The second run covered J81, J82, J159, J160, W9A-8 and W9A-9, and all 6 are killed. Logs: `rows-fix1.log` and `rows-fix1b.log` in the admission-note folder.
- **Row selection:** every row anchored in a changed `src` file, every row whose killer test I changed, every v58-bin row, and W9A-1 to W9A-7. This is wider than the brief's list, to be safe.

## Adversarial review, one line per test
- `sb_6_a_coerce_applied_of_the_wrong_kind_is_refused_at_validate`: reverting to `requested: Some(value.clone())` fails the `Some(19.5)` assertion (W9A-6). Dropping `{value:?}` from the reason fails the `twenty` assertion.
- `sb_6_a_coerce_applied_map_entry_of_the_wrong_kind_is_refused_at_validate`: disabling the applied loop leaves the key unrefused and the preview non-empty (W9A-3 killed).
- `sb_6_a_coercion_applied_of_the_wrong_kind_is_refused_at_validate`: disabling the coercions loop does the same (W9A-4 killed).
- `sb_6_a_prepare_kind_refusal_records_the_specs_request_and_not_the_reported_value`: the reported value in `requested` fails cases (a), (b) and (c) (W9A-7 killed).
- `sb_6_a_prepare_kind_refusal_is_returned_before_a_registered_check_runs`: without the early return the check would panic inside `collect_prepare`, so this test fails by panic rather than at its counter assertion. The Run-level test fires at its named assertions and carries W9A-5. The control (counter 1 on a valid report) shows the counter observes calls.
- `sb_6_a_wrong_kind_report_ends_the_run_with_the_sb_6_violation_not_a_check_panic`: without the early return the reason becomes a KC-30 panic reason, which fails `SB-6:` and `!panick`.
- `ma_12_a_widening_effective_is_refused_at_prepare`, changed: the `test.count` List case now asserts SB-6. A new `ext.` key case keeps MA-12's list branch covered.
  - It asserts `is a scalar` (J159) and `MA-12 (re-match)` (J160), and that no `SB-6:` violation appears, which proves MA-12 was reached.
  - It was a finding of this round: the first row run left J159 and J160 surviving. Send-back 1 fixed it, and both are now killed.
- `kc_24_a_module_emit_is_dispatched_to_its_target`: a restored key restriction would refuse the Emit, so the `p:action:Emit:radio@` line would be missing.
- `kf_03_*` read-back additions: a child Manifest its own reader cannot reconstruct fails `assert_eq!(back, *manifest)`.
- `sb_29_…` (positive pin): dropping the value or an unreadable Manifest fails the `requested` equality or `finish_checked`.

## Trial record
| Run | Launched | Duration | Scope |
|---|---|---|---|
| ds-fix1-1 | 08:24 | ~20 min | F1, F2, F4: production lines, tests, rows W9A-3 to W9A-7 |
| ds-fix1-2 | 08:44 | ~24 min | F3, F5 |
| ds-fix1-3 (send-back 1) | 09:12 | ~6 min | MA-12 coverage restored, J81/J82 retargeted, W9A-8/9 |

- **Send-backs:** one of two used. It was needed because the first row run showed J159, J160, J81 and J82 surviving.
- **What I wrote into tasks:**
  - the exact production lines of F1 and F2 (`exact_request`, the `kind_refused` flag and early return, the reason format);
  - the F3 `run_child` shape, which moves the as-sent hashing into `child_action_as_sent`;
  - the row anchors and mutants;
  - the F4 double split and the test cases with expected values.
- **What I wrote directly:** the `design/03` and `design/06` text edits (F6 and the SB-6 amendments), and the row-selection script.
- **Task files and logs:** `ds-fix1-1-task.md` to `ds-fix1-3-task.md`, `ds-fix1-1.log` to `ds-fix1-3.log`, `rows-fix1.log` and `rows-fix1b.log`, all in `/home/komatsu/.cache/ezsdr-fixes/2026-11-19-admission-note/`.
- **Wall time:** about 1 h 10 min, of which about 15 min was waiting for the sim lock.

## Deviations and INFERRED
- **Decision text changed.** `decisions.md` F2 says "for a coercion, its `requested`". I used the Spec's `Eq` value or `None` for coercion entries as well. A `Coercion.requested` is itself a Module's report (SB-44 only checks the key), so using it would still record a Module's value as `requested`. Please confirm.
- **MA-12 skipped on the SB-6 refusal path.** At prepare the early return also skips MA-12's loops. A Provider with a wrong-kind `test.count` therefore gets only the SB-6 violation. The old `test.count` List case in the MA-12 test could no longer reach MA-12. Its coverage moved to the `ext.` key case (see above).
- **Known gap in the same mechanism.** The MA-12 and SB-44/46 violations in `prepare.rs` still set `requested` to `Some(applied.clone())`, which is a Module's reported value too. That is pre-existing and not a kind violation, so I did not touch it. It may belong in the `requested` note.
- **Unchanged files.** `reactive.rs:118` still calls `RunHandle::finish()` directly, because F5 named only the V58 suite. Other `finish()` calls under `ezsdr-acceptance/tests/` are different types (the `uhd.rs` `Run` wrapper, parity pairs, `governance.rs` `Rehearsal`).
- **Build directory.** I used the worktree's own `.cargo/config.toml` target dir, not `Ez-SDRv4-review`, per AGENTS §7 for worktrees.
- **Flaky test.** `n47s5_front_rehearsal_on_the_fake` failed once under load in DeepSeek's first full run and passed alone and in the later full runs. It is unrelated to this round.
- **Not run:** the Python suites, the Python 3.9 container, hardware, and the 20-run repeat (no new test depends on timing).