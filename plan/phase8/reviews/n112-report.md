# #114 — implementer reports (Claude Sonnet supervising Go DeepSeek)

I fixed the three `requested` producers in `plan/prepare.rs` (MA-12 narrowing, SB-44 preview mismatch, MA-12 re-match). Each now records the Spec's `Eq` value, or nothing for a range, set or unnamed key. It never records the Module's reported value. DeepSeek needed one run and no send-backs. The three new mutation rows were all killed, and the exit checks pass.

**Diff stat against `5cbccee9`** (nothing committed, as briefed):
```
 crates/ezsdr-kernel/src/plan/prepare.rs   |  13 ++--
 crates/ezsdr-kernel/tests/spec_binding.rs | 115 +++++
 design/03-spec-and-binding.md             |   5 +-
 design/05-module-api.md                   |   5 +-
 plan/maintenance/tools/mutations.json     |  30 +++
 5 files changed, 160 insertions(+), 8 deletions(-)
```

**Every `Violation {` producer in `crates/ezsdr-kernel/src`, with its `requested` source:**
- **Changed by this fix, now `exact_request(Some(req), key)`:** the three producers above, at `prepare.rs` ~193, ~222 and ~241. Their reasons now carry the reported value.
- **Already `exact_request` from `6cc8026d`:** the SB-6 kind violations at `prepare.rs:112` and `matching.rs:60`.
- **Always `None`:**
  - `prepare.rs:56`, `prepare.rs:78` (SB-41), `matching.rs:150` (SB-7 stray), `session.rs:315` and `coordinator/pipeline.rs:1199` and `:1209`.
  - `coordinator/admission.rs:247`.
- **The value or constraint the experimenter or the Action supplied** (`violation_with` / `coercion_violation`, `admission.rs:251` and `:255`): these come from the Spec or Action path, not a Module report. I did not audit each of their call sites.
- **Registered checks:** left alone, as the brief says.
- **Still the Module's `c.requested`, not changed:**
  - `prepare.rs:~145`, the SB-46 coercion rejected at prepare.
  - `prepare.rs:~131` and `matching.rs:169`, the SB-44 stray-key coercions at prepare and validate.
  - `matching.rs:169` is not an SB-46 site.
  - `validation.rs:116` and `session.rs:229` and `:237` (SB-46 at validate and at session admission), which read the preview's `Coercion.requested`.

  Issue #114's title also names SB-46, but the brief limited the fix to three producers, so I left these. Test comment `spec_binding.rs:~2455` says the report's own `requested` "is itself a Module report", so these look like the same mechanism. Decision for the orchestrator.

**Tests:**
- All three are in `spec_binding.rs`, and each fails on the old code at its `requested` assertion.
  - `ma_12_a_narrowing_violation_records_the_specs_request_and_the_reported_value`: an Eq request gives `Some(2)` and an unnamed key gives `None`, each with `Int(7)` in the reason.
  - `ma_12_a_rematch_violation_records_the_specs_request_and_the_reported_value`: an Eq request gives `Some(2)` and a `Min` request gives `None`.
  - `sb_44_a_preview_mismatch_records_the_specs_request_and_the_reported_value`: an Eq 19.5 request gives `Some(19.5)`, with `Num(Finite(40.0))` in the reason.
- No existing test pinned the old record. DeepSeek searched `crates/*/tests` for `MA-12`, `SB-44`, `ezsdr.effective` and `\.requested`.
- SB-44 has no non-Eq case because `TestProvider::coerce` only handles `Eq`, so no preview exists to compare.

**Mutation rows `N114-1`, `N114-2`, `N114-3`:** each restores `Some(applied.clone())` for its producer. All three were killed in `rows-114.log`. No existing row's filter selects a changed test, because I changed no existing test.

**Checks:**
- `cargo fmt --all --check` is clean.
- `cargo clippy --workspace --all-targets -- -D warnings` passes (DeepSeek's run).
- `cargo test --workspace`: 3049 passed, 0 failed, 0 ignored (DeepSeek's run). I re-ran `spec_binding` myself: 145 passed.
- `schema_freeze` passes, 5 of 5. No schema moved and nothing was regenerated.
- `rules.py --base 6cc8026d`: 0 problems.

**Adversarial review, one line per test:**
- **Narrowing:** the wrong change `Some(applied.clone())` fails at the `requested` assertion, `Int(7)` against `Int(2)`. The unnamed-key case would also fail, since it expects `None` and the old code gives `Some(7)`. The test picks the violation by reason prefix, so it cannot pass on the re-match violation.
- **Re-match:** the same wrong change fails at the same assertion. The `Min` case expects `None`. Both violations fire and the test selects by prefix.
- **SB-44:** the wrong change fails at `requested`, `40.0` against `19.5`. The test asserts `coercions_preview.len() == 1` and `v.len() == 1`, so it reaches the mismatch branch.
- I did not run a probe for the `None` cases separately. That rests on reading the old code. The probes covered the first assertion of each test.

**Trial record:** one DeepSeek run of about 20 minutes, no send-backs. It put its probe copy under `/tmp/ds114-probe` and it touched nothing outside the task. I wrote the production lines and the design text myself, as briefed; DeepSeek wrote the tests and rows and ran the checks.

**Deviations and inferred:**
- `MA-12` is in `design/05-module-api.md`, not `design/03`, so I edited MA-12 there and SB-44 in `03`. Each has a Changes row, and the MA-12 and SB-44 `*Test:*` lines name the new tests. No issue numbers appear in the rule text.
- I had no reason to doubt whether the SB-44 preview exclusion is keyed per resource. It is, in the existing code.

Files are in `/home/komatsu/local_work/Ez-SDRv4-n112/`, with the task, log and row files in `/home/komatsu/.cache/ezsdr-fixes/2026-11-19-n112/`. Per ponytail, I did not check anything beyond the above, and the SB-46 and stray-SB-44 producers remain an open call.

---

All seven Violation producers that recorded a Module's value as `requested` are fixed, and the SB-46 sites in `session.rs` needed no change. The seven new mutation rows are killed, plus T10, which the edit re-anchored. The exit checks pass. Nothing is committed.

**Diff stat against `5cbccee9`:**
```
 matching.rs       |   6 +-      prepare.rs        |  19 ++-
 validation.rs     |   2 +-      spec_binding.rs   | 235 +++
 tests/support/doubles.rs | 16 ++   design/03 |   9 +-
 design/05         |   5 +-      mutations.json    |  72 +++-
 8 files changed, 346 insertions(+), 18 deletions(-)
```

**Producers, with the rule per site.** The rule is that `requested` is what the experimenter or the Action asked for that key, else `None`; the Module's value goes in the reason.
- **Fixed in round 1, now `exact_request`:** MA-12 narrowing, MA-12 re-match, and SB-44 preview mismatch, all in `prepare.rs`.
- **Fixed this round:**
  - **SB-44 stray key at validate** (`matching.rs:~169`) is now `requested: None`. The key is one the Spec never named, so `None` is right. The reason now carries the reported coercion (`Bool(false)` to `Bool(true)`).
  - **SB-44 stray key at prepare** (`prepare.rs:~131`) is now `requested: None`, for the same reason, with the same reason text.
  - **SB-46 at validate** (`validation.rs:~116`) is now `exact_request(spec.resources.get(&previewed.resource), &c.key)`. The reason already carried the applied value.
  - **SB-46 at prepare** (`prepare.rs:~145`) is now `exact_request(request, &c.key)`. The reason already carried the applied value.
- **Unchanged, `session.rs:229` and `:237` (SB-46 at Session admission):** the `Coercion` there is built by the Kernel from the Action's own value (`coordinator/admission.rs:220`, and `session.rs:339` for the `"asap"` case). Its `requested` is already what the Action asked, so I left it. No existing test pins `requested` for an admission-time `reject`; `rs_17_a_scheduled_rate_change_under_reject_is_refused` asserts only that the failure contains `SB-46`. This rests on my reading of the code: `admit` is public and takes `coercions: &[Coercion]` from the caller, and both callers I found build them in the Kernel.
- **`None` by construction:** `prepare.rs:56` and `:78`, `matching.rs:150`, `session.rs:315`, `pipeline.rs:1199` and `:1209`, `admission.rs:247`.
- **Taken from the Spec or Action path:** `admission.rs:251` and `:255`, plus the SB-6 sites from `6cc8026d`.
- **Left to the check itself, per the brief:** registered checks' own `requested`.

**Tests (all in `spec_binding.rs`).** Each selects its violation by `check` and a reason prefix, and asserts `requested` before the reason. Each fails on the old code at its `requested` assertion, probed in a /tmp copy.
- `sb_44_a_stray_coercion_violation_records_no_request_at_validate`: asserts `None`, and that the reason holds `Bool(false)` and `Bool(true)`.
- `sb_44_a_stray_coercion_violation_records_no_request_at_prepare`: the same assertions at prepare.
- `sb_46_a_rejected_coercion_records_the_specs_request_at_validate`: asserts `Some(19.5)` while the double reports 7.0. I added a small `TestProvider::with_reported_requested` knob to `tests/support/doubles.rs` so the Spec's value and the Module's differ; without it the old code was indistinguishable at validate.
- `sb_46_a_rejected_coercion_records_the_specs_request_at_prepare`: asserts `Some(20.0)` for an Eq request and `None` for `Min(20.0)`, against a hand-built report claiming 7.0.
- **Skipped cases:**
  - **SB-46 at validate, non-Eq:** `TestProvider::coerce` only handles Eq requests, so there is no preview to reject.
  - **SB-46 at prepare, Eq 19.5:** this is unreachable. The default `reject` already refuses the preview at validate, and `collect_prepare` returns that admission's violations before the policy loop runs. The test therefore uses Eq 20.0, which the node satisfies directly.

**Rows.**
- **N114-1 to N114-3 (round 1):** the narrowing, re-match and SB-44 preview rows, all killed.
- **N114-4 to N114-7:** stray at validate (`matching.rs`), stray at prepare (`prepare.rs`), SB-46 at validate (`validation.rs`) and SB-46 at prepare (`prepare.rs`), all killed.
- **T10 re-anchored:** my `prepare.rs` edit changed T10's `old` text, so `rules.py` first reported it matching zero times. DeepSeek re-anchored it to the new token sequence, and it is still killed.

**Texts.**
- SB-44 in `design/03` now says what a stray-key and a preview-mismatch violation record as `requested`.
- SB-46 in `design/03` now says the same for a rejected coercion, including that at Session or schedule admission it is the Action's value.
- MA-12 in `design/05-module-api.md` states the same rule for its two violations.
- The `*Test:*` lines name all the new tests.
- The Changes rows now read `SB-44, SB-46` in `03` and `MA-12` in `05`. No issue numbers appear in rule text.

**Checks.**
- **Passing:** fmt is clean; `rules.py --base 6cc8026d` reports 0 problems.
- **Passing, DeepSeek's run:** clippy with `-D warnings`, and `cargo test --workspace` (0 failures); `schema_freeze` passes and no schema moved.
- **Re-run by me after the last edits:** `coordinator` 164, `run_session` 72 and `spec_binding` 149 passed.
- **Not re-run by me:** the full workspace, after the last edits.

**Adversarial review, one line per new test.** Probe: the old `Some(c.requested.clone())` or `Some(stray.requested.clone())` for that producer, in a scratch copy.
- **SB-44 validate:** the old code returns `Some(false)` against `None`. The test asserts exactly one SB-44 violation, so it cannot pass on another.
- **SB-44 prepare:** the same failure, with `v.len() == 1`.
- **SB-46 validate:** the old code returns `Some(7.0)` against `Some(19.5)`. The test selects by an `SB-46: ` prefix and asserts a count of 1.
- **SB-46 prepare:** the old code returns `Some(7.0)` against `Some(20.0)` or `None`. Both cases assert a count of 1.
- DeepSeek probed all four, and the rows killed the four mutants.

**Trial record.** There were two DeepSeek runs of about 20 and 15 minutes with no send-backs. I wrote all production lines and the design text, and DeepSeek wrote the tests, rows and checks. Its logs and my rows log are in `/home/komatsu/.cache/ezsdr-fixes/2026-11-19-n112/`.

**Deviations and inferred.** `MA-12` lives in `design/05`, not `design/03`, so it is edited there. The two SB-46 tests use Eq 20.0 at prepare for the reachability reason above, which is an INFERRED consequence of `prepare.rs:48-50`, confirmed by the test running.

**Open point for the orchestrator.** `Coercion.requested` still reaches the Manifest as the Module reported it, through `PrepareReport.coercions` and `coercions_preview`. These are Manifest records, not Violations, so I left them. If the same rule should cover them, that is a separate change.

Per ponytail, I did not audit the call sites of `violation_with` and `coercion_violation` in `admission.rs`, and I did not re-run the full workspace tests myself after the final edits.
---

F1–F4 are fixed in the same worktree. All 14 mutation rows in the affected set are killed, and the exit checks pass, including the Python suite (123 tests). Nothing is committed; the diff below is against `99e3afa1`.

**Diff stat against `99e3afa1`:** 8 files changed, 186 insertions, 68 deletions.
- Rust and tests: `plan/coercion.rs`, `plan/prepare.rs`, `tests/spec_binding.rs`.
- Python: `python/ezsdr/_client.py`, `python/tests/test_easy_api.py`.
- Specs: `design/03-spec-and-binding.md`, `design/16-easy-api.md`.
- Rows: `plan/maintenance/tools/mutations.json`.

**F1 (major).** I added `coercion::fragment_request(spec, fragment, key)`. It returns the output's `params` value for a Sink fragment, and `exact_request` over `spec.resources` otherwise. SB-6's `report_kind` and SB-46's policy loop in `prepare.rs` now call it. The MA-12 producers and the validate-stage sites are Provider-only and unchanged.
- **New tests, in `spec_binding.rs`:**
  - `sb_6_a_sink_kind_refusal_at_prepare_records_its_params_value`: asserts `Some(40.0)` for a key the output names (a Sink output whose `params` carries `test.grid = 40.0`) and `None` for a key it does not name.
  - `sb_46_a_rejected_coercion_of_a_sink_records_its_params_value_at_prepare`: a coercion reported as 7.0, expecting `Some(40.0)`.
- **Probes in a /tmp copy:** with the old `exact_request(spec.resources.get(...))` lookup restored, both tests fail at their `requested` assertion, with `left: None`.
- **Rows N114-8 and N114-9:** both killed. I re-anchored W9A-7 and N114-7 to the new call; both are still killed.
- **Texts:**
  - SB-6 and SB-46 in `design/03` now say a Sink fragment's `requested` is its output's `params` value at `prepare`.
  - The two new tests are on the `*Test:*` lines.
  - SB-6's test line went over the 1500-character limit, so I shortened one parenthetical there to "(Sinks too)".

**F2.** `Connection.call` now checks the closed connection first, then encodes, then writes.
- **New test:** `Departure.test_ea_16_a_lone_surrogate_after_an_abandoned_exchange_raises_protocol_error`. With the old order it fails on an assertion (`UnicodeEncodeError … is not an instance of ProtocolError`), not as an escaped error.
- **Row N112-2:** a new row that swaps the order back; killed.
- **Text:** `design/16-easy-api.md` line 127 states the order and names the new test.

**F3.** The #112 test (`test_ea_16_a_lone_surrogate_fails_before_a_request_and_the_session_stays`) catches `Exception` and asserts `UnicodeEncodeError` explicitly, so a restored ASCII-escaping client fails an assertion (probed: `FAIL`, not `ERROR`).
- **Manifest checks:** I replaced the substring searches with checks on the logged entry. The action log holds exactly one entry (the `advance` from `sleep` is not logged): `set_parameter`, with `action["value"] == {"é": 1}` and outcome `rejected`. A recursive walk of every key and value asserts no code point falls in U+D800..U+DFFF.
- **Rows:** N112-1 and C89-3-04 were re-run and are killed.

**F4.** The stray-key `requested == None` and reason assertions (`Bool(false)`, `Bool(true)`) moved into `sb_44_a_coercion_of_an_unrequested_key_is_refused_at_validate` and `..._at_prepare`, selected by check, `SB-44: ` prefix and key `test.flag`. All their old assertions are kept, including the preview-exclusion and warnings checks. I deleted the two duplicate tests and retargeted N114-4 and N114-5 by changing only their `filter`. Probes restoring `Some(stray.requested.clone())` fail both tests at `requested`. The `*Test:*` lines and the Changes rows are updated.

**Rows (`rows-fix.log`):** N114-1 to N114-9, W9A-7, T10, N112-1, N112-2 and C89-3-04: 14 of 14 killed.

**Exit checks:**
- **Passing:** `cargo fmt --all --check` is clean; `rules.py --base 6cc8026d` reports 0 problems.
- **Passing, DeepSeek's run:** clippy with `-D warnings`; `cargo test --workspace` with 0 failures, `schema_freeze` 5 of 5 and nothing regenerated.
- **Re-run by me after the last edits:** all `ezsdr-kernel` test binaries pass (`spec_binding` 149, `coordinator` 164, `run_session` 72), and `python -m unittest tests.test_easy_api` runs 123 tests, OK.
- **Not re-run by me:** the full workspace beyond `ezsdr-kernel`; my last edits were design text and Python only.

**Adversarial review, one line each.**
- **Sink SB-6:** reverting to the resource-only lookup returns `None` against `Some(40.0)`. The count of SB-6 violations is asserted, so another violation cannot satisfy it.
- **Sink SB-46:** the same reversion fails at `requested`, and the Provider's report is asserted violation-free by `v.len() == 1`.
- **F4 stray tests:** restoring the Module's `requested` fails both at `requested` (`Some(false)` against `None`).
- **F2 test:** reversing the check order fails on the exception-type assertion.
- **F3 test:** `allow_nan=False` only gives `ProtocolError` against `UnicodeEncodeError`, a `FAIL`.

**Trial record.** There were two DeepSeek runs (about 20 and 10 minutes) and no send-backs. I wrote all production lines and the design text.

**Deviations and inferred.** Review F5 (the KC-24 pairing) and Sol's note on false `Coercion.requested` in an accepted Manifest were left as your decisions file says. Per ponytail, I did not run the `rdev` or Python 3.9 container variants.