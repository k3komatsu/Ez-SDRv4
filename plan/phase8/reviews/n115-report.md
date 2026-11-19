# #115 and #116 — implementer reports (Claude Sonnet supervising Go DeepSeek)

Both issues are fixed in `/home/komatsu/local_work/Ez-SDRv4-n115` (branch `work/n115`), uncommitted. Every exit check passes except the Python suites, which I did not run.

**Decision for you on #116.** The RF check cannot see the Spec at `prepare`, so there it records no `requested` and names the reported value in its reason. I did not add plumbing. If you want `prepare` to record the Spec's value, the check needs the Spec as an input, which is a SB-30 change. At `validate` it records the Spec's value, and at `runtime` the Action's value.

```
 crates/ezsdr-kernel/src/plan/coercion.rs  |  12 ++
 crates/ezsdr-kernel/src/plan/matching.rs  |  12 +-
 crates/ezsdr-kernel/src/plan/prepare.rs   |   5 +
 crates/ezsdr-kernel/tests/spec_binding.rs | 186 +++++++++++++++++++++++++-----
 crates/ezsdr-radio/src/lib.rs             |  52 +++++----
 crates/ezsdr-radio/tests/channels.rs      |   2 +-
 crates/ezsdr-radio/tests/radio_model.rs   | 123 ++++++++++++++++++++
 design/03-spec-and-binding.md             |   5 +-
 design/07-radio-model.md                  |   5 +-
 plan/maintenance/tools/mutations.json     | 160 +++++++++++++++++++++++--
 10 files changed, 497 insertions(+), 65 deletions(-)
```

## #115: a Module's false `Coercion.requested`
**What changed.** I wrote the production code myself: `coercion::false_request` in `coercion.rs`, with callers in `matching.rs` and `prepare.rs`.
- A reported coercion whose `requested` differs from the experimenter's request for its key is refused as an `ezsdr.coercion` violation.
- The violation's `requested` is the experimenter's value, and the reason names the Module's claim.
- It fires at validate, where the coercion is also kept out of `coercions_preview`, and at prepare. At prepare it fires for a Provider's key (the Spec's `Eq` value) and for a Sink's (its `params` value).
- It applies under every policy, and prepare does not also raise SB-46 for the same coercion.
- A key with no exact request is not checked.

**Tests** (DeepSeek, in `spec_binding.rs`). Each fails on the old code at the assertion its name claims.
- `sb_44_a_coercion_with_a_false_requested_is_refused_at_validate`: old code admits it under `accept`.
- `sb_44_a_coercion_with_a_false_requested_is_refused_at_prepare`: covers `reject`, `accept` and a Session's `warn`.
- `sb_44_a_coercion_of_a_sink_with_a_false_requested_is_refused_at_prepare`
- `sb_44_a_coercion_of_a_key_with_no_exact_request_is_not_checked`: a `Min` request keeps the Module's 7.0. It passes on old and new and pins the boundary.

**Existing tests changed.** Three SB-46 tests, `sb_30_prepare_runs_the_checks_against_the_applied_configuration` and `sb_6_a_prepare_kind_refusal_records_the_specs_request_and_not_the_reported_value` used a lying Module to tell the Spec's request from the report's. Their coercions now report the Spec's own value, and the comments now say a differing claim is SB-44's. The `Min` part of the prepare test keeps its 7.0.

**Rows** `N115-1` to `N115-7`. All 7 killed, plus `N114-7`, `N114-9` and `W9A-7`, which select changed tests (10/10 passed).
- `N114-6` is deleted: its mutant is equivalent now that SB-44 stops a lying Module, so it can no longer be killed.
- With `N114-6` gone, nothing pins SB-46's validate-time `requested` for a non-`Eq` key, because `TestProvider` coerces only `Eq`.

## #116: the RF-envelope check's `requested`
**What changed.** I wrote this production code too, in `crates/ezsdr-radio/src/lib.rs`.
- At `validate`, `requested` is the configuration's value, which is the Spec's.
- At `runtime`, it is the Action's value, and absent for a key the Action does not name.
- At `prepare`, it is absent.
- Reasons now name the checked value where they did not: "not a number", "not a string", and the enable violation ("its stream is …"). One existing assertion in `channels.rs` was updated for the enable reason.
- Clippy flagged `check_channel_antenna` at 8 arguments, so I folded `is_rx` and `number` into one `channel` tuple parameter.

**Tests** (DeepSeek, in `radio_model.rs`):
- `rm_19_a_prepare_violation_records_no_requested_and_names_the_reported_value`: fails on old code at the `requested is_none` assertion.
- `rm_19_a_validate_violation_records_the_specs_value`: pins `validate`, so it passes on old code too.
- `rm_19_a_runtime_violation_records_only_what_the_action_asked`: fails on old code at c1's `requested is None`.

**Rows** `N116-1` to `N116-8`, plus `P59`, `P60`, `P61` and `N47S4-19`: all 12 killed.

## Texts
- `design/03-spec-and-binding.md` SB-44: the new obligation, five test names, and a Changes row.
- `design/07-radio-model.md` RM-19: what `requested` means at each stage, three test names, and a Changes row.
- MA-11 does not state what `coerce` returns for `requested`, so I left it alone.

## Checks
- `cargo fmt --all --check`: ok.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: exit 0, 74 test-result lines all ok, including `schema_freeze` 5 and `spec_binding` 153. No in-tree Provider or Sink trips the new SB-44 refusal.
- `rules.py --base b92f8916`: 0 problems.
- Not run: the Python suites, and the full mutation lists and gate (yours).
- EA-16: the refusal appears as an ordinary `ezsdr.coercion` violation and fires only on a Module bug, so no easy-API text needed to change.

## Adversarial review, one line per test
- `sb_44_..._at_validate`: deleting the push, or dropping the preview exclusion, fails the refusal and the "not recorded" assertions respectively. Rows `N115-1` and `N115-2` killed.
- `sb_44_..._at_prepare`: replacing the request with `None` or deleting the `continue` fails the "only violation, not also SB-46's" assertion. Rows `N115-3` and `N115-4` killed.
- `sb_44_..._of_a_sink_...`: using `exact_request` instead of `fragment_request` leaves SB-46's violation instead of SB-44's. Row `N115-5` killed.
- `sb_44_..._no_exact_request_is_not_checked`: treating `None` as a mismatch refuses the report. Row `N115-6` killed.
- Comparison-flip probe: it fails the validate test's refusal assertion. Row `N115-7` killed.
- `rm_19_a_prepare_violation_...`: the Prepare arm and each of the four reason texts are killed by `N116-1` and `N116-5` to `N116-8`.
- `rm_19_a_validate_violation_...`: the Validate arm and dropping `requested` are killed by `N116-2` and `N116-4`.
- `rm_19_a_runtime_violation_...`: the Runtime arm is killed by `N116-3`. Assertion (b) was checked on old code, and (c) was checked separately in a copy with (b) removed, because (b) fails first.

## Trial record
- DeepSeek was launched twice, once per issue, with no send-backs. Each time it wrote tests, rows and probes in a /tmp copy.
- It left no stray files, and all its probes were in /tmp copies.
- It flagged the clippy failure on my own code, as I had told it not to touch `crates/ezsdr-radio/src`.
- I wrote the production lines, the design text, and the removal of row `N114-6` myself.

## Deviations and INFERRED
- INFERRED: the radio check at validate always sees the Spec's request. `requested_violations` builds the configuration from `Eq` values and output `params`.
- INFERRED: SB-46 at Session admission still records the Action's value, as in the previous step, and I left it unchanged.
- The enable violation now carries "(its stream is …)" in its reason. That goes slightly beyond the brief, to keep the reported value visible at `prepare`.
- The production code uses `.expect("a channel judged has a stream")`, which holds because the loop `continue`s earlier when the stream is missing.

Records for the orchestrator's landing are in `/home/komatsu/.cache/ezsdr-fixes/2026-11-19-n115/`: `ds-1.log`, `ds-2.log`, `rows.log` (first run), `rows2.log` (second run), and the two task files.

---

F1, F2 and F3 are fixed in `/home/komatsu/local_work/Ez-SDRv4-n115`, uncommitted on top of HEAD. All exit checks pass.

The #115 and #116 report above still stands. Only the changes in this round are listed here.

**Decision for you.** The F1 override is by key only, and fragments are not told apart. A violation for another fragment that names the same key as the Action's would also get the Action's value as `requested`. I judged it unreachable in practice, since another fragment's applied value was already admitted earlier. Say if you want the override restricted to violations the check itself marked as coming from `proposed`.

## F1: a registered check's `requested` at runtime
- **Production** (mine, `AdmissionCheck` interface unchanged):
  - `Admitter::admit_asked(..., asked: Option<&BTreeMap<Key, Value>>)` in `crates/ezsdr-kernel/src/session.rs` overwrites each registered check's violation `requested`. It becomes the Action's own value for the key the violation names, or `None` for any other key. The coercion-policy violations (SB-46) are untouched.
  - `admit(...)` calls it with `None`, so the three tests that call `admit` directly keep their behaviour.
  - Runtime admission in `coordinator/admission.rs` builds `asked` from the Action before step 10's coercion and passes `Some(&asked)`.
- **Tests:**
  - `v58_16_a_runtime_refusal_records_what_the_action_asked_not_the_coerced_value`: Sol's case, run through Session submission. It asserts `requested == Some(2.9)`, a reason containing `gain 3 dB exceeds 2.95`, and the gain staying 0. On the pre-fix code it fails at `requested`, 3.0 versus 2.9.
  - `kc_24_a_registered_check_s_violation_records_what_the_action_asked`, part (a): the ceiling check on the snapped `test.grid`. It fails on the old code at `requested`, 20.0 versus 19.5.
  - Part (b): a check that names another key with `Some(1.0)` gets `None`. It fails on the old code at `Some(1.0)` versus `None`.
- **Rows:** `N115F-1` (delete the overwrite), `N115F-2` (pass `None` instead of `Some(&asked)`), `N115F-3` (another key keeps the check's value) and `N115F-4` (the acceptance twin of `N115F-2`). All killed.

## F2: `N114-6` retargeted
- **Test:** `sb_46_a_rejected_coercion_of_a_key_with_no_exact_request_records_none_at_validate`.
  - Setup: `Present(test.flag)`, `with_stray_coercion()`, and policy `Reject` for `test.flag`.
  - It asserts one SB-46 violation, no SB-44, key `test.flag`, `requested == None`, and a reason containing `Bool(true)`.
- **Row:** `N114-6` is restored from `b92f8916` with the new filter. It was killed: the mutant gives `Some(false)`.

## F3: wording
- **Test comment:** the SB-30 test comment now says `coerce` is called (SB-7) and returns no coercion for 20.0, so nothing is previewed. DeepSeek also found the same wrong claim in `sb_44_a_provider_may_not_exempt_a_key_by_declaring_a_coercion`. That test was not changed by this branch, so I reported it and did not fix it.
- **RM-19 `*Test:*` line:** it now says what each stage test establishes.
- **Wording that follows from F1:**
  - RM-19's runtime clause: "the Action's own value before a Provider's coercion, which the Kernel records".
  - KC-24 step 12 and SB-30: a short paragraph on the new rule, kept off the long lines because `rules.py` caps their growth.
  - `Admitter::admit_asked` is named in step 12 and in the SB-30 table of checks by stage.
  - New Changes rows in specs 03 and 06.
- **Left alone:** RS-17 in `design/04-run-and-session.md` still names `Admitter::admit`, which exists.

## Checks
- `cargo fmt --all --check`: ok.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: exit 0, 74 test-result lines all ok, no FAILED. `schema_freeze` is part of it.
- `rules.py --base b92f8916`: 0 problems.
- `remote_mutate.py` on the five rows: 5/5 killed.
- **Process:** DeepSeek ran once with no send-backs. All its probes ran in a /tmp copy, and it changed no existing test.

Files are in `/home/komatsu/.cache/ezsdr-fixes/2026-11-19-n115/`:
- `ds-fix-1-task.md`
- `ds-fix-1.log`
- `rows-fix.log`