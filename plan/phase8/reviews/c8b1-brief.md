# 8b first clean-up (c8b1) — briefs

## brief (first round)

### 8b first clean-up (lane 2, before the 2026-11-27 bench) — Sonnet, may drive Go DeepSeek

Worktree `/home/komatsu/local_work/Ez-SDRv4-n111`. Create branch `work/c8b1` from `main` (`0159b54c`) with `git switch -c work/c8b1 main` — the only branch operation you may do. Then do not commit, stash, push, reset or switch branch. Never touch `/home/komatsu/local_work/Ez-SDRv4` (another lane). Work dir: `/home/komatsu/.cache/ezsdr-fixes/2026-11-20-c8b1/`.

**Risk: not risky** (test infrastructure, a no-op cleanup in the TX path, removing two retired hardware tests, adding pure-rule tests). No test plan. Terra reviews the diff. Read `plan/phase8/52-phase8-end-review.md` §2b, §2d rows 12, 30, 32, and §4's D1-trial line (the `ezsdr-radio` gaps).

## Items
1. **Row 12 — `Direct` has no `Drop`** (`crates/ezsdr-radio-uhd/tests/fake.rs:1379`, used ~52 times): give it a `Drop` that ends its Run — stop and join even while panicking, errors discarded (§2b line ~253) — so a failing test does not leak threads or a Run. Check `UhdRadio` (§2b line ~236 says it has none either): add one only if the same leak applies and it is small; otherwise say why not.
2. **Row 30 — the TX header's `contract` take and restore** (`crates/ezsdr-radio-uhd/src/provider/tx.rs` ~170, 231, 790, 817; 47s2 P1169): remove the useless take/restore so the code reads the field in place, with no behaviour change. Say why it is a no-op (who else could observe the field between take and restore). Existing tests and rows must still pass; retarget any row anchored on the removed text to the same decision.
3. **Row 32 — `hw_m1` / `hw_m2`** (`crates/ezsdr-radio-uhd/tests/hardware.rs:1489, 1533`): they test the withdrawn RM-34 path (front end moved between streamers). Confirm from `design/07` (RM-34's stub) and the code that the path they exercise no longer exists or no longer means what they assert; if so delete them (their results stay in `plan/phase8/bench-results.md`) and any row, helper or runbook line that only they use. If either still tests live behaviour, keep it and say what.
4. **The D1 trial's `ezsdr-radio` gaps** (note 52 §4, the trial in `plan/phase8/reviews/tools2-trial.md`): add table tests in `crates/ezsdr-radio` for the missed mutants it lists that are real gaps — `Grid::contains` / `nearest` / `at_or_above` / `at_or_below` at their edges, `x3xx.rs` `decimation`, `FrontEnd::receives` (true for `BasicTx`/`LfTx` is wrong), `uhd_name`, `addressed`'s edges, `RfEnvelopeCheck` with `lo_hz == hi_hz`. Pure decisions, table tests at that level. Confirm each test kills its mutant by running the sweep on that file or function through `simq` (`plan/maintenance/tools/remote/sweep_mutants.py`, e.g. `run radio sim02:1 -- --re <fn>`; read its docstring) — and add hand-written rows only where a test alone would not keep it pinned in the gate (the gate runs hand-written rows, not the sweep): one row per decision, smallest anchor.

## Rules
- Mutation rows only through `remote_mutate.py` via `simq`; the sweep only via `sweep_mutants.py` via `simq`. Never run `mutate.py` locally or apply a mutant to a tree. Build output never in the tree; temp files on /tmp. Kill by PID only. chmod 644 files you create.
- DeepSeek, if used: tasks `ds-<n>-task.md`, logs `ds-<n>.log` in the work dir; a task file must not start with `---`; launch `cd /home/komatsu/local_work/Ez-SDRv4-n111 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`; real PID via `pgrep -n -f 'deepseek-v4.1-flash#max'`, check `/proc/<PID>/cwd`; wait with `timeout 590 tail --pid=<PID> -f /dev/null` (Bash timeout 600000); review its diff (≤ 2 send-backs).
- Before reporting: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, the UHD feature clippy (`plan/maintenance/tools/remote/rdev.py -- cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`), `cargo test --workspace` (remote via rdev is fine), `python3 plan/maintenance/tools/rules.py --base 0159b54c` (with `--renames` if you rename tests), the touched and new rows through `simq` + `remote_mutate.py`.

## Report
Japanese, short: diff stat; each item (what, why no behaviour change for 2, the evidence for 3); the sweep's before/after for 4; rows and verdicts; INFERRED; open questions.

## brief (fix round)

### 8b first clean-up — fix round (last) — Sonnet, implement directly

Worktree `/home/komatsu/local_work/Ez-SDRv4-n111`, branch `work/c8b1` at `23d8cc72`. Same rules as `brief.md`. Do not commit, stash, push, reset or switch branch.

Terra's review (`terra.md` beside this file), both majors adopted (orchestrator):
1. **Row 30 reverted.** The take/restore of the TX header's `contract` exists to avoid a `String` allocation per block on the TX path. Restore the original code in `crates/ezsdr-radio-uhd/src/provider/tx.rs` exactly (`git diff 0159b54c -- crates/ezsdr-radio-uhd/src/provider/tx.rs` must be empty) and add one comment at the take saying why (no allocation per block). No other change there.
2. **`Direct`'s Drop catches a cleanup panic only while the thread is already unwinding** (`std::thread::panicking()`); on the normal path a cleanup panic fails the test. Add a test for the normal path if one is cheap (a cleanup that panics without a prior panic fails the test, e.g. via `catch_unwind` around a scope that drops a rig whose cleanup is made to panic) — if the rig cannot be made to panic in cleanup without new production hooks, skip the test and say so.

Rows: rerun the C8B1 rows and a sample of `fake` rows that use `Direct` (as before) through `simq` + `remote_mutate.py`; the 235 `tx.rs` rows need no rerun once `tx.rs` is back to `0159b54c` exactly. `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p ezsdr-radio-uhd --test fake`, `python3 plan/maintenance/tools/rules.py --base 0159b54c`.

Report (Japanese, short): diff stat; each item; rows.
