# Re-check of the #112/#114 fix round (843f9a26), Opus, read-only

Scope: `git diff HEAD~1 HEAD` and the step `6cc8026d..HEAD` in `/home/komatsu/local_work/Ez-SDRv4-n112`.

## Own failure scenarios

I drafted these after reading the step diff, which already contained the fix, and before reading sol-code.md, decisions.md and report-fix.md. They are not independent of the fix: scenario 1 is Sol's F1, and scenario 8 overlaps Sol's F2 and F3.

1. A Sink fragment's prepare refusal (SB-6 kind, SB-46 reject) takes `requested` from `spec.resources`, which has no entry under the Sink's id, so the result is `None` and not the output's `params` value.
2. A Provider fragment and an output share an id, so `fragment_request` gives the Provider the output's params value.
3. An Executor fragment (`island_<n>`) gets some value as `requested` when it should get `None`.
4. A Sink with no params, or a key its params do not name, gets a non-`None` `requested`.
5. A Sink output and a Provider both name key `k`, and one fragment's violation carries the other's value.
6. The Session or schedule SB-46 path uses a coercion that a Module built.
7. A remaining `requested: Some(<module value>)` site in the Kernel, or in a registered check, at validate or prepare.
8. In the client, encoding before the closed check (gives UnicodeEncodeError, not ProtocolError, after a departure). Encoding inside the abandon `try`: a surrogate closes the Session. Dropping `ensure_ascii=False`: the surrogate is escaped and kills the server. A raw newline in the UTF-8 header.

Results:
- 1 is fixed (F1).
- 2 cannot happen. SB-22a puts resources, components, outputs and Island ids in one namespace, refused at validate (`crates/ezsdr-kernel/src/plan/validation.rs:143-162`), and `collect_prepare` refuses any admission that is not admitted and not this Run's (`prepare.rs:47-55`).
- 3: `island_<n>` is in neither `outputs` nor `resources`, so the lookup gives `None` (`coercion.rs:23-28`).
- 4: `params.get` gives `None`. The second half of the Sink SB-6 test asserts this.
- 5 is correct by construction, because the lookup is keyed by the fragment id (note N2).
- 6 is fine. `admit` receives only Kernel-built coercions: the Action's value at `coordinator/admission.rs:220`, and `"asap"` at `session.rs:341`. The Module origin is not coerced (`admission.rs:140-143`).
- 7: one site remains, outside the Kernel (note N1).
- 8 is fine. `json.dumps` escapes control characters even with `ensure_ascii=False`.

## Findings of sol-code.md

- **F1 (major): CLOSED.**
  - `fragment_request` (`crates/ezsdr-kernel/src/plan/coercion.rs:23`) returns the output's `params[key]` for a Sink fragment. For every other fragment it returns `exact_request(spec.resources.get(fragment))`.
  - SB-6 at prepare uses it (`prepare.rs:111`), and so does SB-46 at prepare (`prepare.rs:147`).
  - The remaining prepare sites are Provider-only by construction:
    - The SB-44 stray check runs only under `spec.resources.get(fragment)` (`prepare.rs:127-136`). Its `requested` is `None`.
    - MA-12 narrowing, the SB-44 preview mismatch and the MA-12 re-match iterate `spec.resources` and use `exact_request(Some(req))` (`prepare.rs:195,229,248`).
  - Validate sites:
    - SB-46 at validate is Provider-only (`validation.rs:119`).
    - The SB-44 stray at validate gives `None` (`matching.rs:171`).
    - SB-7/SB-44 in matching uses `exact_request` (`matching.rs:60`).
  - Executor fragment: `None`. Sink with no params: `None`. Sink and Provider sharing a key: each fragment reads its own map.
  - Tests:
    - `sb_6_a_sink_kind_refusal_at_prepare_records_its_params_value` (`tests/spec_binding.rs:2642`) asserts `Some(40.0)`, plus `None` for an unnamed key.
    - `sb_46_a_rejected_coercion_of_a_sink_records_its_params_value_at_prepare` (`:2103`) asserts `Some(40.0)` against the reported 7.0, with `v.len()==1`.
    - Rows N114-8 and N114-9 put back the resource-only lookup. Both are killed (`rows-fix.log`).
  - Re-run here: 21 tests of `spec_binding` under the filters `sb_6_`, `sb_46_`, `sb_44_` and `ma_12_` pass. `rules.py --base 6cc8026d` reports 0 problems. `cargo fmt --check` is clean.
- **F2 (minor): CLOSED.**
  - `Connection.call` checks the closed connection first, then encodes, then enters the abandon `try` (`python/ezsdr/_client.py:78-92`).
  - The test `Departure.test_ea_16_a_lone_surrogate_after_an_abandoned_exchange_raises_protocol_error` (`python/tests/test_easy_api.py:1835`) asserts the exception type explicitly. N112-2 puts the old order back and is killed.
  - Re-run here: passes.
- **F3 (minor): CLOSED.**
  - The #112 test (`test_easy_api.py:143`) catches `Exception` and asserts `UnicodeEncodeError`. Removing `ensure_ascii=False` then gives an assertion FAIL (`ProtocolError` is not a `UnicodeEncodeError`). N112-1 is killed.
  - The Manifest check now asserts:
    - exactly one logged entry;
    - kind `set_parameter`;
    - value `{"é": 1}`;
    - outcome `rejected`;
    - by a code-point walk, that no surrogate appears.
  - Re-run here: passes.
- **F4 (minor): CLOSED.**
  - The `requested == None` and reason assertions now live in the two existing SB-44 stray tests (`spec_binding.rs:3422`, `:3459`). Each test keeps:
    - its earlier refusal assertion;
    - the preview-exclusion and warning checks (validate);
    - `v.len()==1` (prepare).
  - The duplicate tests are gone. N114-4 and N114-5 point to the existing tests through `filter` only, and both are killed.
- **F5 (note):** KC-24 pairing; the orchestrator left it unchanged as KC-24 states. Not re-opened.

## New findings

- **N1 (minor; pre-existing, not introduced; read, not run): RM-19's registered check records a Module's reported value as `requested` at prepare.**
  - Sites: `crates/ezsdr-radio/src/lib.rs:439,442,450,453,462-468,482,486`, through `push_numbered_violation` at `:496`.
  - At prepare the check reads each fragment's `effective` (SB-30's second point; `design/07-radio-model.md:208`).
  - Sequence:
    1. The Spec asks `radio.tx.c0.frequency_hz` `Eq(2.4e9)`, inside the allowed band.
    2. The Provider's `prepare` reports `effective` 5.8e9, outside every band, together with `radio.tx.c0.stream` ≥ 0. Without that key the check skips the channel (`lib.rs:425-429`).
  - Wrong outcome: two violations for one key with different `requested` values:
    - MA-12's re-match: `requested: Some(2.4e9)`.
    - RM-19's: `check: radio.rf_envelope`, `requested: Some(5.8e9)`, the Module's value.
  - Severity: minor, because no `design/` rule defines RM-19's `requested` and this site is a Vocabulary check outside the seven Kernel producers. It is still an eighth producer that contradicts the invariant the brief states, and #114's claim to cover every producer.
  - The design texts this step changes cover only the Kernel's producers (SB-6, SB-44, SB-46, MA-12), so no `design/` rule is broken. The subject of commit 99e3afa1 ("every prepare and validate violation ...") over-claims, though. Narrow it in the squash message, or file RM-19's `requested` at prepare as a follow-up. It is the owner's or the orchestrator's choice whether a registered check should follow SB-6's meaning.
- **N2 (note): no test has a Sink output and a Provider naming the same key.**
  - The code is right by construction: the lookup goes by fragment id, and SB-22a forbids a name shared across the sets (`coercion.rs:23-28`, `validation.rs:143-162`).
  - Gap: a wrong lookup that finds `key` in any output's params, or that falls back to any resource's `Eq`, would pass every current test. Each fixture has a single output, and the Provider asks only `test.count`.
  - Optional: give the Provider `test.grid Eq 19.5` in one of the two Sink tests, and assert the Provider's own violation (if one exists) carries 19.5.
- **N3 (note): no mutation row pins the core of #112, that encoding stays outside the abandon region.**
  - The wrong change: move `header = json.dumps(...).encode(...)` inside the outer `try` (`_client.py:85-99`).
  - The #112 test already kills this on an assertion. Trace of the loop at `test_easy_api.py:155-161`:
    1. The first value `"\ud800"` raises `UnicodeEncodeError`, and the moved encode then abandons the Session.
    2. The second value `{"\udfff": 1}` hits the closed check and raises `ProtocolError`.
    3. `except Exception` catches it, and `assertIsInstance(caught, UnicodeEncodeError)` fails.
  - Rows N112-1 and N112-2 do not name this placement. A row N112-3 is optional.
- **N4 (minor; introduced by the round): SB-6's `*Test:*` line misattributes Sink coverage.**
  - It now ends the Executor test's entry with "(Sinks too)" (`design/03-spec-and-binding.md:156`, SB-6 Test line).
  - `sb_6_an_executor_prepare_effective_of_the_wrong_kind_is_refused` (`crates/ezsdr-kernel/tests/coordinator.rs:404`) binds only an Executor. It drives no Sink.
  - The parenthetical it replaced said a Sink's report takes the Executor's path in `collect_prepare`. This round made that false, because a Sink's `requested` now comes from `fragment_request`.
  - The Sink claim is already covered by the new `sb_6_a_sink_kind_refusal_at_prepare_records_its_params_value`, named on the same line. Fix: delete "(Sinks too)".

## Evidence that each new test fails on its named assertion

I did not apply any mutant (the brief forbids it). I read the probe output in `ds-fix-1.log` and `ds-fix-2.log`, and each new or changed test fails on the assertion it names:

- Putting back the resource-only lookup:
  - The Sink SB-6 test panics at `spec_binding.rs:2666`, the `requested` assertion: `left: None` (`ds-fix-1.log:3106-3108`).
  - The Sink SB-46 test panics at `:2128`, the same assertion: `left: None` (`ds-fix-1.log:3168-3170`).
- Putting back `Some(stray.requested.clone())`:
  - The validate stray test panics at `:3447`: `left: Some(Scalar(Bool(false)))` (`ds-fix-1.log:3286-3288`).
  - The prepare stray test panics at `:3492`, with the same `left` (`ds-fix-1.log:3348-3350`).
- Python:
  - Reversing the order gives an AssertionError: "UnicodeEncodeError … is not an instance of ProtocolError" (`ds-fix-2.log:1635`).
  - Removing `ensure_ascii=False` gives an AssertionError: "ProtocolError … is not an instance of UnicodeEncodeError" (`ds-fix-2.log:1682`).

No major findings. The fix round introduced no regressions: the step tests I re-ran pass, `rules.py` is clean and fmt is clean. I did not re-run the full workspace or the full Python suite.
