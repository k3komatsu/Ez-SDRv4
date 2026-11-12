# Note 51 stage M — review record and dispositions (2026-11-12)

Note 51 §7.2 row M with issue #91: a mutation row's outcome is read from the test output, not the exit code. Risk: not risky (tooling); Sol reviewed the code because the gate's meaning changes. Branch `work/n51m` from `4f2c5b9`, landing on `main` as one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n51m-briefs.md](n51m-briefs.md)).

## What it became

- `mutate.py`: `killed` only for a test failure with no `EZSDR-INFRA:` marker, no Cargo `(signal:` and no timeout; otherwise `ERROR` (broken at the gate), or `killed (bound)` for a row that declares `"kill": "bound"` (Q8). The result is a fixed field (`EZSDR-ROW {json}`, with `why` for a non-killed outcome), read by `remote_mutate.py` and `gate.py` (#91: a row name containing ": killed").
- `infra(msg)` / `timing(msg)` markers in `crates/ezsdr-radio-uhd/tests/common/infra.rs`, one `write_all` each; the watchdog writes its marker before `abort()`.
- Creation-time classification (§6.2) in `session_on`, `spec_run`/`start_spec` (§5.3a N3) and `UhdRig::with_run`: a Run ended at creation by KC-12a, KC-21a, RS-8a or a join timeout writes one marker and panics once.
- Q16 (b″): a failure whose failed tests (by package, binary and path) all carry a timing marker is re-run alone, at most twice, by `gate.py`'s test step and by `mutate.py` while the mutant is applied; the first conclusive re-run decides.
- Q9: `FakeDevice::new` prints `EZSDR-CLASS: real-clock` on the real clock when the runner asks; `gate.json` reports `real-clock (unclassified)` (185 rows; report-only until stage 3).
- `remote_mutate.py --e2e`: fixtures 1, 2, 3, 3b, 5 and two timing fixtures run in the mutation image through the same `cargo_test()` as the rows; `gate.py` has an `e2e` step.

## The full run's ERROR rows (fix round 1, decided by the orchestrator)

The first full run under the new verdicts: 2145/2151; five `ERROR` rows (plus the known survivor `phase7/G09`).
- `phase6/G21` (infra marker): a new assertion in `ea_14_refusals_before_a_child_runs` refuses the over-bound duration before the child runs.
- `maintenance/E2A2A-01` (test timeout): retargeted to the pure `txbook_outcomes_give_the_cursors_the_executor_gives_today`.
- `maintenance/P1119` (infra marker): new test `ur_23_g1b_a_refused_priority_call_does_not_latch_the_guard`.
- `phase7/C06` (signal: a double panic in the Run's drop): `kc_29_…` leaks the Run before panicking, so the assertion is the failure.
- `phase7/G16` (test timeout; "the wait has no deadline"): declared `"kill": "bound"`, the only declared row.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, code | 3 | 2 | `failure-collapsed-into-value` (majors 1–2: a failure excused by a later pass or by counting) |
| Opus, re-check | 0 | 3 | — |

## Dispositions

**Sol, code (FAIL)** — all adopted ([n51m-sol-code.md](n51m-sol-code.md)).
- Major 1, the gate re-ran a conclusive failure until it passed: fixed, the first re-run that is not timing-marked is final (`settle_test`, shared by `mutate.py` and `gate.py`).
- Major 2, re-run eligibility compared counts, so an unrelated failure or another binary's SIGKILL could be excused: fixed, failed tests are matched to markers by identity; any infra marker, signal or missing result line keeps the failure.
- Major 3, `spec_run` did not read the Manifest at creation: fixed, `start_spec` shares `creation_end` with `session_on`; callers that expect an ended Run take `Err(manifest)`.
- Minor 4 (a marked pass counted as a pass) and minor 5 (`why` from the wrong invocation): fixed. Note: fixture 3 now requires KC-12a and `FAILED`.

**Opus re-check (majors closed)** ([n51m-recheck.md](n51m-recheck.md)) — minors adopted, fixed without another review:
- Markers matched by path only: now by package, binary and path.
- `#[ignore]` hardware callers of `spec_run` now panicked on a refused Run: `hw_perf_rx_rates` uses `start_spec` and prints the refusal as before; the others keep `spec_run` (they need a running Run).
- No test drove `spec_run`'s creation path with a bound: fixture 3b.
- Note: the gate's own re-run path is covered by string self-tests only.

## Left to later stages (note 51 §7.2)

Fixtures 4, 4b, 6, 7 and the classification on every wrapper method are stage 2a; the 11 other creation sites of §5.4 are stage 2a; the real-clock marker becomes enforcing after stage 3.

## Gate

PASS ([n51m-gate.json](n51m-gate.json)) on sim02:7 and sim03:7: mutation 2171/2172 (`phase7/G09` known), `killed (bound)` 1 (`phase7/G16`), no `ERROR`, no timing re-runs, 187 rows select a real-clock test (report-only); e2e 9/9.
