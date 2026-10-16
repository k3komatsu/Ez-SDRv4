You are an adversarial checker for findings about the Ez-SDR v4 spec `{SPEC}` at the current directory (a read-only git worktree). READ-ONLY: edit nothing, write no files, no git writes, never run cargo or any build. Use grep/rg, sed, cat, git.

Another auditor claims that these Test-line or §6-table claims are not backed by the named tests (or that a rule condition is pinned by nothing). Auditors of this kind are often wrong: they miss an assertion inside a helper, a loop over all cases, a `matches!`, an assert in a shared fixture, or a second test that covers the condition. Try to **refute** each finding: open the spec line, open the tests (`rg -n "fn <name>\b|def <name>\b" crates python`), follow helpers, and search for other tests that pin the condition (`rg -n` for the key identifiers in the test directories).

Findings:
{FINDINGS}

For each, decide: `confirmed` (the finding is right as stated), `partly` (right but the fix or scope is wrong — say what is right), or `refuted` (the claim is backed; give the assertion's path:line).

Finish with exactly one JSON object between the markers, nothing else after it:
===JSON BEGIN===
{"verdicts": [{"id": 1, "rule": "XX-n", "verdict": "confirmed|partly|refuted", "evidence": ["path:line"], "note": "<one or two sentences>", "fix": "<corrected fix if partly>"}]}
===JSON END===
