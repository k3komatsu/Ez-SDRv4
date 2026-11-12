# Note 51 stage 2a — Opus re-check of fix round 1 (2026-11-13; read-only, Claude Opus 5.5 effort high; nothing built or run)

**Major CLOSED.**
1. `judge_end` (crates/ezsdr-radio-uhd/tests/common/run.rs:65-86) reads `section.reason`, every `also` entry, a timed-out `cleanup_failures` entry (RS-8a) and the join notes before any declaration (VERIFIED). KC-21a is recorded only at coordinator/paced.rs:186-198, KC-12a only at pipeline.rs:582,629 through `fail()`; both go through `request()`, which puts a failure after another end into `also` (ending.rs:216-226), copied to the Manifest (ending.rs:155). RS-8a only at run.rs:588; the join note at provider/mod.rs:405 under node `usrp`. No declaration hides a bound (`Expect` is consulted after a bound returned; `join_is_subject` removes only the final join and RS-8a outranks it; `rx_join_issue_67` drops only `uhd-rx-*` notes); a non-bound later end stays `Fine` (run.rs:456). The real race is not driven end to end (INFERRED; a microsecond window); the pure test covers the decision and `Run::judge` passes `&manifest.termination` directly.
2. `capture_markers`: `mark` keeps a marker only if `captures(expected, limit)`; others go to `infra` (run.rs:107-122). The four bounded callers pass `&["join"]` and assert `met join` and one marker; the two no-bound tests no longer capture (VERIFIED).
3. The scan (governance.rs:509-511): no false positives in the real tree (every other `connect` token is in a string the scan blanks); a glob import then `connect(` and a macro body are caught (VERIFIED).
4. Rows N51-2A-04, -17, -26, -27, -28, -40, -41, -42: anchors unique, each mutant intended, each dies on a plain assertion (VERIFIED by reading; outcomes from fix1-mut.log).

New, none major:
- Minor (INFERRED): N51-2A-42's test pins only the pure `captures` helper; a `mark` that skipped it and captured every marker would survive.
- Note (VERIFIED path, no instance): `support/mod.rs:61` `session_run_with(.., assembly, ..) -> RunHandle` is in a directory the scan skips; `uhd.rs` imports `spec_run` from it for a Mock Run, so a FakeDevice Run could be created through it unchecked. Pre-existing.
- Note: tests/README.md:101 still said the scan fails "a raw `connect(`"; fixed by the orchestrator.
