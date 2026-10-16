You audit one slice of the Ez-SDR v4 specs at the current directory (a read-only git worktree). READ-ONLY: edit nothing, write no files, no git writes, never run cargo, rustfmt or any build. Use grep/rg, sed, cat and git to read.

## Your slice
Spec `{SPEC}`, rules: {RULES}.

## The question
Each rule ends in a `*Test:*` line (older specs: a `*Checked by …*` sentence; or `*Producer obligation*`) naming the tests that pin it, sometimes with a parenthesis saying what a test checks, and sometimes "none named for …". The spec's §6 (or "Tests") table has rows `| test | input | expected | rules |`. **A Test line or table row must claim only what the named test really asserts.** Reviewers keep finding claims a test does not back (a test said to check "every value" that asserts three; a test said to pin a Provider's behaviour that never runs that Provider; a row's "expected" naming a flag the test never inspects).

For each rule in your slice:
1. Read the rule and its Test line, and every §6 row whose `rules` column names it.
2. For every test named there, find its body (`rg -n "fn <name>\b|def <name>\b" crates python`), read it fully, and follow the helpers it calls far enough to know what is actually asserted (`assert*`, `expect`, `unwrap_err`, `matches!`, comparisons, `#[should_panic]`). Note whether it exercises the component the rule is about (the right Provider/crate/path), not a stand-in.
3. Classify each claim:
   - `pinned`: the test asserts it (give the assertion's path:line).
   - `overclaim`: the Test line, its parenthesis or a §6 row's expected/input says the test checks something it does not assert, or the test exercises something else.
   - `wrong_test`: a named test does not exist, or does not relate to the rule at all.
   - `silent_gap`: a normative condition of the rule (a refusal, a "never", an "only", a number, an order) that no named test asserts and the Test line does not list under "none named". Report only clear, specific conditions, not paraphrase.
   - `stale_none_named`: the line says "none named for X" but a named test (or one you found) asserts X.
Be precise and conservative: a claim backed by an assertion in a helper the test calls is `pinned`. If unsure, say `pinned` and explain in `note`. Do not report style.

## Output
Finish with exactly one JSON object between the markers, nothing else after it:
===JSON BEGIN===
{"spec": "{SPEC}",
 "rules_ok": ["<rule ids with no finding>"],
 "findings": [
  {"id": 1, "rule": "XX-n", "where": "<spec path:line of the Test line or §6 row>", "claim": "<the claim, quoted exactly>",
   "tests": ["<test fn names involved>"], "verdict": "overclaim|wrong_test|silent_gap|stale_none_named",
   "evidence": ["<path:line or path:start-end of the test code you read>"],
   "note": "<what the test actually asserts, one or two sentences>",
   "fix": "<the narrowed wording, or the 'none named for …' to add>"}
 ]}
===JSON END===
Every rule id of your slice appears either in `rules_ok` or in at least one finding. Paths are repository-relative.
