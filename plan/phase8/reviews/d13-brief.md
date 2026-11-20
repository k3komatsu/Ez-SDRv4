# D13 conversion (lane 1, before the 2026-11-27 bench) — Sonnet, may drive Go DeepSeek

Main tree `<main tree>`. Create branch `work/d13` from `main` (`11a1314e`) with `git switch -c work/d13 main` — the only branch operation you may do. Then do not commit, stash, push, reset or switch branch. Never touch `<worktree>` (another lane). Work dir: `~/.cache/ezsdr-fixes/2026-11-20-d13/`.

**Risk: not risky** (a conversion script for bench records; no production code). No test plan; write a test with it. Terra reviews the diff.

## The decision (owner 2026-11-13, D13; read `~/.cache/ezsdr-fixes/2026-11-13-stage4/decisions.md` line 15 and `proposal.md` §2 "P-admission" and §4 D13; `plan/phase8/reviews/stage4-proposal.md` is the same text in the tree)
Records placed in `crates/ezsdr-radio/measurements/` (read by `crates/ezsdr-acceptance/tests/measurements.rs:21` — every placed record binds the profile, the test compares all of them against the Mock and UHD descriptions) are only: produced by the current measurement groups (no removed key, no deleted group), `load: none`, one per (motherboard, layout), the latest clean one at the stage-4 closing commit. Everything else stays in `bench/`. A conversion script writes `observations: []` and stamps the real `selector.args`.

## What to do
1. Read the measurement record format: `crates/ezsdr-radio/src/measurement.rs`, `crates/ezsdr-radio/tests/measurement.rs`, the record-suite writer (`crates/ezsdr-radio-uhd/tests/common/s4bench.rs`, `tests/common/measurement/`), and an existing record-suite output under `bench/` (e.g. `bench/2026-10-28-record-suite/`, `bench/2026-11-16-stage4/`) — find which file the bench writes and what differs from what `measurements.rs` accepts.
2. A script `plan/maintenance/tools/place_measurement.py` (or the name the existing tools use for this kind of thing; check `plan/maintenance/tools/` first): input one record-suite output file plus its bench `conditions.json`; refuses a record that fails P-admission (a removed key or a deleted group — derive the current set from the code, not a hand list, if practical; `load` not `none`; a second record for the same (motherboard, layout) already in the directory unless `--replace`); writes the placed file with `observations: []` and the real `selector.args`; prints what it did. Python 3, stdlib only, `PYTHONDONTWRITEBYTECODE=1`.
3. A self-test (`--self-test`, like `rules.py`'s) with small fixtures on /tmp covering each refusal and one accepted record; and, if a current-format record exists in `bench/` that passes P-admission, a dry run on it whose output `cargo test -p ezsdr-acceptance --test measurements` accepts when copied into a temp copy of the measurements directory — do **not** place any file in `crates/ezsdr-radio/measurements/` (that happens after the 11-27 bench, from that day's records).
4. `plan/phase8/bench.md`: one short step in the record-suite section saying how the day's records are placed (the command), and that only the closing-commit `load: none` records per (board, layout) go in.

## Rules
- Build output never in the tree; temp files on /tmp. Kill by PID only. Never `chmod -R`; chmod 644 files you create in the tree (and 755 only for a script that must be executable, if the other tools are).
- DeepSeek, if used: tasks `ds-<n>-task.md`, logs `ds-<n>.log` in the work dir; a task file must not start with `---`; launch `cd <main tree> && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > <log> 2>&1 < /dev/null &`, real PID via `pgrep -n -f 'deepseek-v4.1-flash#max'`, check `/proc/<PID>/cwd`; wait with `timeout 590 tail --pid=<PID> -f /dev/null` (Bash timeout 600000); review its diff (≤ 2 send-backs).
- Before reporting: `python3 plan/maintenance/tools/rules.py --base 11a1314e`, the script's `--self-test`, `cargo test -p ezsdr-acceptance --test measurements`.

## Report
Japanese, short: diff stat; the format differences found; the refusals and how the current group/key set is derived; self-test and dry-run results; INFERRED; open questions with your recommendation.


## brief-fix

# D13 fix round — Sonnet, implement directly, plus one investigation

Main tree `<main tree>`, branch `work/d13` at `d68a4df4` (your work, committed). Same rules as `brief.md` beside this file. Do not commit, stash, push, reset or switch branch.

Terra's review: `terra.md` beside this file. Dispositions (orchestrator):
1. **Deleted groups by word match (major): adopted.** Compare whole condition (group) identities: derive the current set of condition ids the record-suite writers produce (their literal ids, or their generated forms written out explicitly, e.g. a per-rate id from the rate list), and refuse a record whose condition id is not in it. Self-test: a deleted group made entirely of surviving words is refused.
2. **A transient-failure record (major): adopted.** A record whose `observations` is non-empty (a stopped ladder, a blocker, any failed trial) is refused with the observations printed; the operator takes a clean record instead. `--accept-observations "<reason>"` overrides and the reason goes into the script's printed summary and into the bench README step text (say so in bench.md); the placed file still has `observations: []` per D13. Self-test both ways.
3. **Numbers rewritten (major): adopted.** Only `selector.args` and `observations` may change: parse numbers losslessly (e.g. `parse_float`/`parse_int` keeping the source text, or edit the two fields without re-serialising the rest) and add a self-test that the placed `values` (and every other field) equal the input exactly, with a value like `9007199254740993.0`.

**Investigation (report, do not fix):** the dry run of the 11-13 R1 record fails `crates/ezsdr-acceptance/tests/measurements.rs` on (a) `tx_bytes_per_s` (profile 8e8, measured 4e8) and (b) the `rx.s0` / `tx.s0` `sample_rate_hz` grids. Terra (terra.md, last two paragraphs) says (a) is not only staleness: TX at 200 MS/s underflows host-fed on both boards (`bench/2026-11-13-n67-records/README.md:13`, `bench/2026-11-16-stage4/README.md:17`, KL-5) while `crates/ezsdr-radio/src/x3xx.rs:420` declares 800 MB/s. Find, with path:line: what the measured key means (the writer's ladder in `tests/common/measurement/multichannel.rs`) vs what the profile value means and what `measurement.rs`'s `rule()` compares; whether a fresh 11-27 record would fail the same way; and for (b), the exact failing grid points (run the dry run again with the test's full output, print both grids) and why. End with one recommendation (e.g. a rule change, a KL entry, a different measurement) and its alternatives — the orchestrator brings it to the owner if it is a measured value or a user-visible limit.

Checks: the script's `--self-test`, `python3 plan/maintenance/tools/rules.py --base 11a1314e`, the dry run in a /tmp host copy as before. Test temp files on /tmp. Never place a file in `crates/ezsdr-radio/measurements/`.

Report (Japanese, short): diff stat; each item; the investigation's findings with evidence and the recommendation.


## brief-fix2

# D13 fix round 2 — Sonnet, implement directly

Main tree `<main tree>`, branch `work/d13` at `4272fea6` (your fix round, committed). Same rules as `brief.md`. Do not commit, stash, push, reset or switch branch. Your investigation's report is the basis (the R1 dry run fails on (a) `tx_bytes_per_s` and (b) the sample-rate grids).

1. **(a) Owner decision (2026-11-20):** `radio.perf.tx_bytes_per_s` is not compared: make it `NotCompared` in `crates/ezsdr-radio/src/measurement.rs`'s `rule()` (`measurement.rs:299`), with the reason in one line (the profile's 8e8 is the admission bound; sustained host-fed TX is KL-5's Ceiling; the measured value stays in the bench record). Update the design text that states the comparison rule for this key (search `design/` for the measurement comparison rules and `tx_bytes_per_s`; one Changes row in that spec, no issue numbers), and KL-5 in `KNOWN_LIMITATIONS.md` (one clause: the sustained TX measurement is recorded but not compared against the profile). A test that a record with `tx_bytes_per_s` below the profile passes, and the mutation row(s) that test kills; the place_measurement key set must still match (`NotCompared` keys are part of its derivation — check).
2. **(b) The 1-ulp float reading:** first **verify the user-visible side** without changing code: does a Python Session request carrying a rate like `200e6/14` (JSON `14285714.285714285`) reach the Rust server 1 ulp off, so that an X3xx rate on the `Values` grid (`crates/ezsdr-radio/src/device.rs:57`, exact `contains`) is missed and coerced or refused? Write a small Python check in /tmp against the in-tree server (Mock x3xx; `EZSDR_SERVER` built from this tree; see AGENTS §7 for the venv) and report the exact outcome (accepted / coerced with the recorded values / refused) — do not add it to the tree yet. Then the fix: enable `serde_json`'s `float_roundtrip` feature where it reaches every crate that parses JSON in production and tests (the workspace's crates share one build, so features unify; prefer one declaration — check whether a `[workspace.dependencies]` entry exists or declare it on `ezsdr-kernel`'s dependency, keeping its `OV-15: never enable preserve_order` comment). Check what it changes: canonical hashes of fixtures or golden files (`cargo test --workspace` on sim via `plan/maintenance/tools/remote/rdev.py -- cargo test --workspace`), schema freeze, the Python suites. Add a Rust test that a 17-digit float (e.g. `14285714.285714285`) parses to the exact `200e6/14`, and a row that removes the feature... (a Cargo.toml row may not be practical: if not, say so and pin it by the test alone). If the Python check showed a user-visible miss, also add that sequence as a Python test.
3. Rerun the dry run of the 11-13 R1 record in the /tmp host copy: report what still fails, if anything.

Checks: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (remote via rdev is fine), the Python suite (`plan/maintenance/tools/gate.py` runs it; locally per AGENTS §7), `python3 plan/maintenance/tools/rules.py --base 11a1314e`, place_measurement's `--self-test`. Mutation rows through `simq` + `remote_mutate.py` (log `mut-fix2.log` here). Test temp files on /tmp.

Report (Japanese, short): diff stat; (a) with the design/KL text; (b) the user-visible verification result first, then the fix and what it changed; the dry run; rows.


## brief-fix3

# D13 fix round 3 — Sonnet, implement directly (simplify, do not add heuristics)

Main tree `<main tree>`, branch `work/d13` at `f4325ecb`. Same rules as `brief.md`. Do not commit, stash, push, reset or switch branch.

Terra's re-check: `terra-recheck.md` beside this file (FAIL: majors 1–4, problem 5). Dispositions (orchestrator) — the placement is a manual step done once per bench by the orchestrator, so the tool should make the exact checks and show the evidence, not guess:

1. **Current code = an exact commit check, replacing the condition-id patterns (Terra 3, and the earlier word/pattern checks).** Refuse a record unless the files that define what a record contains are unchanged between the record's own commit (the record carries it — find the field) and `HEAD`: the record-suite writers (`crates/ezsdr-radio-uhd/tests/common/measurement.rs`, `tests/common/measurement/`), `crates/ezsdr-radio/src/lib.rs` keys and `crates/ezsdr-radio/src/measurement.rs` (`git diff --quiet <commit> HEAD -- <paths>`; a commit not in HEAD's history is refused). Remove the condition-id pattern lists and their self-test cases (the generated-id code is then unneeded); keep the exact key-set check. Self-test with a temporary git repo on /tmp, or by injecting the diff function — your choice, smallest.
2. **Raw records only (Terra 1).** Refuse a record without an `observations` field (a `bench_extract.py` copy) with a message naming the raw `measurement.json` to use.
3. **Observations: show all, ask once (Terra 4).** Print every observation in full. `not_compared_*` observations are expected and need nothing; any other observation requires `--reviewed "<note>"` (rename `--accept-observations`), whose note is printed for the bench README. bench.md's step says the same.
4. **Keep the TX result in the retained bench copy (Terra 2).** `plan/phase8/tools/bench_extract.py` keeps `not_compared_*` observations in its copy (the other observations may stay stripped as today, unless keeping them all is simpler — say which). Check KL-5's and design/18's "stays in the bench record" then holds.
5. **Placeholder collision (Terra 5).** The lossless number handling must not touch strings: refuse an input whose text already contains the placeholder token, or use a token that cannot occur in JSON text; self-test with Terra's example.

Checks: `--self-test`, `python3 plan/maintenance/tools/rules.py --base 11a1314e --renames ~/.cache/ezsdr-fixes/2026-11-20-d13/renames-fix2.txt`, bench_extract's own tests if any, and the dry run in the /tmp host copy as before (with the R1 record's commit check expected to refuse it — show that, then the copy with a forced pass to confirm the measurements test still accepts). Temp files on /tmp.

Report (Japanese, short): diff stat (expect smaller); each item; runs.


## brief-fix4

# D13 fix round 4 (last) — Sonnet, implement directly

Main tree `<main tree>`, branch `work/d13` (round 3 committed). Same rules as `brief.md`. Only `plan/phase8/tools/` and `plan/phase8/bench.md`. Do not commit, stash, push, reset or switch branch.

Terra's re-check 2 (`terra-recheck2.md` beside this file). All adopted; this is the last round, no further review (orchestrator):
1. `bench_extract.py` marks its copy (one field, e.g. `"bench_copy": true`) and `place_measurement.py` refuses a marked record, naming the raw `measurement.json`.
2. `DEFINES` adds the record-suite entry point `crates/ezsdr-radio-uhd/tests/hardware.rs` (and any helper it calls that shapes the record — check, list what you added). The tool also refuses when any DEFINES path has uncommitted changes in the working tree (`git diff --quiet HEAD -- <paths>`), so the key set it reads is HEAD's.
3. With `--replace`, serialize and validate the new record fully before removing or replacing the old file; write via a temp file and `os.replace`. Self-test: a refused replacement leaves the old file intact.

Checks: `--self-test`, `python3 plan/maintenance/tools/rules.py --base 11a1314e --renames ~/.cache/ezsdr-fixes/2026-11-20-d13/renames-fix2.txt`. Temp files on /tmp.

Report (Japanese, short): diff stat; each item.
