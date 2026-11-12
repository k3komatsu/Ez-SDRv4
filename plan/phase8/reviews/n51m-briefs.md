# ===== brief.md

# Brief — note 51 stage M: honest mutation verdicts (with #91)

Main tree /home/komatsu/local_work/Ez-SDRv4, `main` at `4f2c5b9`. You are the implementer (AGENTS.md §9). Risk: **not risky** (tooling) — no test plan, no self-review round (waived for this step); Sol reviews the code at the end because the gate's meaning changes.

Read first: plan/phase8/51-test-timing.md §5.4 (the part on creation), **§6 in full**, §7.2 row M, and the appendix (owner decisions Q8, Q9, Q13, Q14, Q16 (b″), and the section "段階 0 の表の後"); plan/phase8/51-tables.md §4 (bound list, esp. §4.4) and §6 (rows); GitHub issue #91 (`gh issue view 91`); AGENTS.md §7 and §9.

## What to build (note 51 §7.2 row M; nothing from 1a–4)

1. **Verdict from output lines** (§6.3 table) in `plan/maintenance/tools/mutate.py` (today `:141-148` turns every non-zero exit into `killed`). Outcomes: `killed` only on a test failure line (`test result: FAILED`, Python `FAILED (`) with no `EZSDR-INFRA:` and no Cargo `(signal:`; `ERROR` for an infra marker, `TEST_TIMEOUT`, a signal without marker, a failure with no `test result:` line, a build timeout. A row with `"kill": "bound"` (Q8) gets `killed (bound)` for the marker and `TEST_TIMEOUT` cases instead of `ERROR`. Results go in a **fixed field**, not parsed out of a free-text line that can contain the row's name (#91; today `remote_mutate.py:33` `ROW` regex, which `gate.py` imports). `gate.py` treats `ERROR` as broken.
2. **`infra(msg)`** (§6.3): one helper in the UHD test support (`crates/ezsdr-radio-uhd/tests/common/`) that writes `EZSDR-INFRA: <test name>: <msg>\n` with a single `std::io::stderr().write_all`, then the caller panics (or aborts in the watchdog). The watchdog (`tests/common/watchdog.rs:53-58`) uses it before `abort()`.
3. **Creation-time classification** (§6.2 points 1–2, limited per §7.2 to creation): `session_with` (`tests/common/mod.rs:~494`), the helper `spec_run` (`tests/common/mod.rs:~460`: §5.3a N3 moves it into M and **overrides** §5.4/§7.2, which list it among the 12) and the `UhdRig` constructor (`crates/ezsdr-acceptance/tests/parity.rs`). On return, if the Run is `CleanedUp`, take the Manifest (`finish()`), and if the reason starts `KC-12a:`/`KC-21a:` or `cleanup_failures` has a `timed_out: true` row (RS-8a) or the Provider recorded `did not join within`, call `infra` and panic once; otherwise the existing assertion. `UhdRig`'s constructor: its real-time bounds (`DeviceAuthority::new` 5 s, …) cut with a marker. The prefix classification is a **pure function with its own unit test** (`KC-12a:`, `KC-21a:`, `timed_out`, `did not join within`, other). The other 11 creation sites of §5.4 and every non-creation method are stage 2a — do not touch them.
4. **`"kill": "bound"`** (Q8): accept the field in `mutations.json`, and in whatever validates rows (`rules.py`/`anchor.py`, if they check fields). Declare **no row yet**; declarations come from the full run (below).
5. **Real-clock marker** (Q9, report-only): `FakeDevice::new` prints `EZSDR-CLASS: real-clock` (plain `eprintln!` is fine) when its clock is Real **and** an env var the runner sets is present. The runner runs each row's baseline with `--show-output` and records which rows select a real-clock test; `gate.json` reports the count and ids as `real-clock (unclassified)`. Nothing fails on it in this stage.
6. **Q16 (b″) runner re-runs**: a test that fails with a timing marker (choose one string, e.g. `EZSDR-TIMING:`, written the same way as `infra`, and carrying what an exact re-run needs: package, test binary and the full test path) is re-run one at a time — by `gate.py`'s test step after the whole `cargo test` run, and by `mutate.py` right after that row's test invocation **while the mutant is still applied** (the mutant is restored when the row ends) — (`--test-threads=1`, `--exact` filter), at most 2 times. Gate test step: passes if a re-run passes with no timing marker. Mutation row: a re-run that fails without a timing marker kills; one that passes does not kill; timing marker on both re-runs → `ERROR`. Counts in `gate.json`. The marker text itself tells a manual `cargo test` user the serial re-run command. No test emits this marker yet (stage 2c); the fixtures below exercise it.
7. **`mutate.py --e2e`** (§6.4): fixtures `infra_fixture_*` in the `fake` binary, gated by `EZSDR_INFRA_FIXTURE=<name>` (return at once otherwise; not `#[ignore]`), run through the **same `cargo_test()`** the rows use, verdicts fixed. Stage M fixtures: **1** (assert failure → killed), **2** (stopped clock + `EZSDR_TEST_WATCHDOG_SECS=1` → marker + `(signal:` → `ERROR` / `killed (bound)`), **3** (Sol's sequence: creating the Run on a device whose prepare never returns → KC-12a → one marker, one `panicked at` → `ERROR`; try first `fake_real` with a long `apply_delay` (decided in the stage 0 tables); only if that cannot reach KC-12a, add the smallest fault and say why), **5** (`abort()` without marker → `ERROR` even for `bound`), plus timing fixtures (they need a test that emits the timing marker; fixture-only): always timing-marked → `ERROR`/fails, and timing-marked once then passing (state in a dir the e2e driver passes by env) → passes. `--e2e` runs **inside the mutation image on a sim host, through the same `remote_mutate.py` → `mutate.py` → `cargo_test()` path as the rows** (that is its point: probe 7's image behaviour is INFERRED until then), and `gate.py` runs it and checks its verdicts; local runs alone do not count. Fixtures 4, 4b, 6, 7 are stage 2a — not now.
8. **Self-tests**: `mutate.py`'s self-test covers every cell of the §6.3 table and the re-run rule as string cases; `gate.py`'s self-test (today `:555` pins `killed (timeout)` as a pass) is changed to the new outcomes.
9. **Docs the change makes false**: tool docstrings, `crates/ezsdr-radio-uhd/tests/README*` if it describes the watchdog/markers, `plan/maintenance/failure-mechanisms.md` only if a row's text becomes false. List (do not edit) lines in AGENTS.md and handoff.md that become false; the orchestrator edits those.

## Expected red

After this stage, rows that today pass as `killed (timeout)` or are killed by a marker (G16 and E2A2A-01 at least; the full run will show more) become `ERROR` until the orchestrator declares them `bound` or they are retargeted. That is the intended outcome — do not work around it, and do not declare rows yourself.

## Rules

- Edit only the tooling under `plan/maintenance/tools/` (and `remote/`), the UHD test support and fixtures in `crates/ezsdr-radio-uhd/tests/`, `parity.rs`'s `UhdRig` constructor, `FakeDevice::new` (and a fault if fixture 3 needs one), `mutations.json` (no row changes except re-anchoring if formatting moves a token). No production behaviour change beyond the env-gated marker.
- Builds and tests: locally or `plan/maintenance/tools/remote/rdev.py -- <cmd>`. Mutation rows only through `remote_mutate.py`; run only rows whose filter or killer you touched (e.g. rows selecting `parity.rs`/`session_with` tests), on **`sim03:4` only** (another lane may need compute). **Do not run the full list** — the orchestrator runs it after your report; a second run of yours will then settle each `ERROR` row.
- `cargo fmt --all`; `python3 plan/maintenance/tools/rules.py --base 4f2c5b9` must pass; `PYTHONDONTWRITEBYTECODE=1`; temp files on /tmp; kill processes by PID only. No commit.
- If something in note 51 cannot be built as written, stop that item and say why in the report rather than inventing a mechanism.

## Report (final message)

What each of items 1–9 became (file:line), the e2e fixtures' observed outputs and verdicts, the rows you ran and their outcomes, the list of AGENTS/handoff lines now false, and anything left undone with its reason.

# ===== brief-fix1.md

# Brief — note 51 stage M, fix round 1: settle the ERROR rows of the full run

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51m` (your stage M work is committed locally as `8df51c2`; the step's start is `4f2c5b9`). Your first brief: /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51m/brief.md (its rules bind). Implementer: OpenCode Sonnet 5.5 #high.

The orchestrator's full run (`remote_mutate.py all sim02:7,sim03:7`, log mut-full1.log in the same directory): 2145/2151. Not passed: `maintenance/E2A2A-01`, `maintenance/P1119`, `phase6/G21`, `phase7/C06`, `phase7/G16` — all `ERROR` — and `phase7/G09` (`SURVIVED`, a known survivor; leave it). 185 rows report `real_clock: true` (report-only; nothing to do).

1. **Make an ERROR say why.** The row line carries the reason: `"why"` = `build timeout`, `test timeout`, `infra marker`, `signal`, `no test result line` (only for non-killed outcomes). Self-test it. Without it an ERROR cannot be settled from the log.
2. **Settle each of the five** (run them with `remote_mutate.py <list> sim03:4 <ids>`; one list per call is fine): from the reason and the test, decide per row and say why:
   - the mutant hangs and the row's only kill is the bound (e.g. G16, whose name says "killed by the per-mutation timeout") → declare `"kill": "bound"`;
   - a cheap device-time or observation assertion would kill it without a bound (note 51 §6.5: "kill by not coming" against a device-time bound first) → add that assertion to the filter's test (or retarget the row to a test that has it), keep the row an assertion row;
   - the ERROR is load or flakiness, not the mutant (re-run twice; same verdict?) → say so with the evidence; do not declare.
   Q8: declared rows are kept few — prefer the second option where it is cheap and does not need a wall-clock bound.
3. Nothing else changes. `cargo fmt --all`, `rules.py --base 4f2c5b9` = 0, `rdev -- cargo test --workspace` green if you touched Rust. No commit.

Report: per row, the reason seen, the decision and its evidence, and the rows' final outcomes on sim03.

# ===== brief-fix2.md

# Brief — note 51 stage M, fix round 2: Sol's code review

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51m` (local commits `8df51c2`, `7ad8f65`; step start `4f2c5b9`). Implementer: OpenCode Sonnet 5.5 #high. Your briefs brief.md and brief-fix1.md (same directory) still bind. Read Sol's review sol-code.md there. All findings adopted (decided by the orchestrator):

1. **Major 1** (gate.py:409-416): in the timing re-run rule, a re-run that fails **without** a timing marker is conclusive: stop and keep the failure; only a re-run that is again timing-marked allows the next attempt. Same rule in mutate.py's `judge` (check it is already so; make both share one rule if that is the simpler code).
2. **Major 2** (gate.py:404-416, mutate.py:205-208): match markers to the **identities** of the failed tests, not counts. Re-run only when every failed test carries a timing marker; any failed test without one, and any signal/infra/no-`test result:` failure anywhere in the output (another binary's SIGKILL included), is kept and never excused by a passing re-run.
3. **Major 3** (common/mod.rs:473-480): `spec_run` classifies creation like `session_on` — on `CleanedUp`, take the Manifest and apply `creation_limit` with `cleanup_failures`' `timed_out` and the join note. For the callers that expect an ended Run back (`fake.rs:1641`, `:1653`, `:4857`), give them a helper that returns what they assert on (termination, and the Manifest if they read it) after the classification; change their assertions only in spelling. No Kernel change.
4. **Minor 4** (mutate.py:215-217): a re-run passes only if it is marker-free.
5. **Minor 5** (mutate.py:183-190, 264-266): `why` is the decisive re-run's own reason (signal, build timeout, infra marker …), not a blanket "re-run unsettled" — keep "re-run unsettled" only for two timing-marked re-runs.
6. Sol's note: fixture 3 also requires `KC-12a` in the marker and `FAILED`; and add string self-test cases for the gate's re-run path (majors 1–2 counterexamples, Sol's SIGKILL case).

Run the rows whose killer you change (`remote_mutate.py <list> sim03:4 <ids>`; the other lane may run mutation on both hosts — if so, wait for it to finish rather than overlap). `cargo fmt --all`; `rdev -- cargo test --workspace` green; self-tests of mutate.py and gate.py; `rules.py --base 4f2c5b9` = 0; `remote_mutate.py --e2e sim03` passes. No commit.

Report: per finding, the change (file:line) and how a test pins it; the e2e and rows run.

# ===== brief-fix3.md

# Brief — note 51 stage M, fix round 3: the re-check's minors

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51m` (head `1380b90`; step start `4f2c5b9`). Implementer: OpenCode Sonnet 5.5 #high. Your earlier briefs (brief.md, brief-fix1.md, brief-fix2.md in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51m/) bind. The Opus re-check of fix round 2 is recheck-fix2.md there. Adopted (decided by the orchestrator):

1. `rerunnable` (and the gate's use of it) matches a timing marker to a failed test by **package, binary and test path**, not the path alone. Add the re-check's counterexample (binary `c`'s `t_c` fails unmarked, binary `b`'s `t_c` marker) as a self-test case that keeps the failure.
2. Audit every `#[ignore]` caller of `spec_run` in `crates/ezsdr-radio-uhd/tests/hardware.rs` (re-check names :216, :252, :1095, :1152, :2780, :3596; search for all). A caller that handled a Run ended at creation (printed it and went on, e.g. `hw_perf_rx_rates` over refused rates) uses `start_spec` and handles the `Err(manifest)` exactly as before (same printed text); a caller that never expected that keeps `spec_run`. The hardware tests' behaviour on the bench must stay as before except that a real-time bound at creation now shows the infra marker. List each caller and what you chose. Compile with the `uhd` feature through rdev.
3. Add e2e fixture 3b: the same creation-time KC-12a driven through `spec_run`/`start_spec` (not `session`), same expected verdict (`ERROR`, one marker with KC-12a, `FAILED`).

`cargo fmt --all`; `rdev -- cargo test --workspace` green; `rdev -- cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets` clean; self-tests; `rules.py --base 4f2c5b9` = 0; `remote_mutate.py --e2e sim03` all cases ok (the other lane's gate may be using sim02/sim03: the e2e is small, run it anyway). No commit. Report per item.

