# Brief — #89 S3: Python client, KL-43, bench tooling

Repository /home/komatsu/local_work/Ez-SDRv4, branch `work/n89`. The head is the S2 commit; the orchestrator names its hash at launch. The step's start is `8dd1016`. S1 (`676ca5d`) added `RunHandle::disconnect_signal`. S2 added the watched server input, A2, the Detached refusal, `BrokenPipe` exit 0, and a failed stdin read as a departure.

Read first, in /home/komatsu/.cache/ezsdr-fixes/2026-11-11-n89/:
- brief.md (its rules bind);
- sketch.md ("Owner decisions" 1–10 first, then §2.3, §3 and §6.3);
- fable-design.md (F4, F5, F6, F7);
- test-plan.md (the S3 section, both "Changes after …" sections, and the S2 fix-round paragraph);
- plan-adjudication.md and plan-dispositions.md (all of it binds).

Then run `git log --oneline 8dd1016..HEAD` and `git show --stat HEAD`.

## Scope (the test plan's S3)

1. **`python/ezsdr/_client.py`**
   - `Popen(..., start_new_session=True)`.
   - Abandon on any exception while a reply is awaited. The `try` covers the write as well as the read of the header line and of the body. On abandon, close both of the server's pipes, mark the connection, and re-raise.
   - Error *replies* abandon nothing.
   - Later calls raise `ProtocolError`.
2. **`python/ezsdr/session.py` `close()`**
   - After an abandon: send no `finish`, wait for the server's exit (the 30 s wait, then kill, as today), then read `<dir>/manifest.json` if it can. A missing or partial Manifest gives `manifest = None` and no raise.
   - `close()` can be called again.
   - When `close()`'s own `finish` fails, it raises `ProtocolError` as today.
   - Fix the existing fallback's order: wait for the exit, then read.
   - `connect()` still closes on any exception.
3. **Text**
   - EA-16 (`design/16-easy-api.md`): an interrupted call ends the Session; `close()` after an abandon; distinguish an earlier abandoned call from `close()`'s own `finish` failing.
   - EA-16's `lease=`, and the `connect`/`Session` docstrings in `python/ezsdr/session.py` (around :976): the stdio server now refuses a Detached Lease (EA-10, S2). Make the text say so.
   - Docstrings of `sleep`, `wait_until`, `wait_for`, `capture`, `result` and `close`.
   - Wherever user-facing text describes S2's departures (read error → exit 0; A2: nothing is served after the input ends), it must match EA-15 as written in S2.
4. **KNOWN_LIMITATIONS.md: a new entry, KL-43.** Use the format of the other entries.
   - It covers:
     - silent ssh partitions;
     - signals sent to the server itself, and its death (no cleanup, no Manifest; the transmit stops only when the device runs out of samples, INFERRED: E9 is to be measured);
     - F4: with `start_new_session`, Ctrl-C no longer kills a hung server; a second Ctrl-C during Python's 30 s wait leaves it running; a remote server behind ssh is never killed by the client;
     - F5's three additions;
     - "a Manifest exists when the server had begun the request".
   - Workaround: `ClientAliveInterval`/`ClientAliveCountMax` in the sshd of whichever host runs the server over ssh. Write it generically; the owner uses many PCs. Also stop the client, not the server.
   - Status: Ceiling (owner 2026-11-11). The A1 backstop is to be decided after the E9 bench measurement.
5. **Bench tooling**
   - `plan/phase8/tools/ssh-drop-driver.py`: `EZSDR_DROP_WAIT=time|sleep`, default `time`.
   - `plan/phase8/tools/ssh-drop-tx.sh`:
     - pass the wait mode through;
     - `EZSDR_DROP_CUT=link|kill` (`kill` mode kills the server container instead of the ssh client, and prints an E9 measurement line: the time from the kill to the end of the tone on #2's monitor, instead of C1/C2);
     - the request-entry relay with the ≥ 1 s rule (otherwise C1/C2 read INCONCLUSIVE).
   - Every verdict and measurement line is machine-printed, never typed.
   - `plan/phase8/bench.md`: a new section "#89 の後" with three runs:
     - (i) `sleep` at `8dd1016`, FAIL expected;
     - (i) `sleep` at the new commit, PASS expected;
     - (ii) `kill`, measuring E9.
   - That section also covers the splitter wiring, the attenuator confirmation before any transmit, the hold margin (F7), and how to read E9 off the old commit's emergency stop.
   - Check `bash -n` on the script, and add a self-check of the driver's mode parsing where the existing tools have one.
6. **Tests:** the Python tests P1–P7 and the change to `test_ea_16_errors`, exactly as the plan states (P6's wrapper `read` mode; P7 mocks `TimeoutExpired`, with no real 30 s wait). Run them under both Python 3.14 and 3.9.
7. **Mutation rows:** C89-3-xx as the plan states, Python and shell rows with exact-substring anchors. Each names its killer in `filter`.

## Binding prohibitions (learned on earlier steps)

- **Mutation rows**
  - A re-anchored row stays in its own file and Provider.
  - No two rows are identical.
  - Every `new` must run or compile.
  - Name a killer only after you have checked it: apply the mutant in a /tmp copy outside the repository and run the killer there.
  - Never use mutate.py or remote_mutate.py, and never apply a mutant inside the tree.
- **Governance**
  - Do not edit governance or PO/OV rules (`plan/phase1/00-overview.md`, `plan/phase2/00-overview.md`, `crates/ezsdr-acceptance/tests/governance.rs`) without asking the orchestrator first. If you need such an edit, stop and report.
- **Files and tests**
  - New files get mode 644.
  - Python runs with `PYTHONDONTWRITEBYTECODE=1`.
  - Use the suite command gate.py uses: `python -m unittest discover -s python/tests`, with `EZSDR_SERVER` pointing at the server built from this tree. Run it under 3.14 through `python3 plan/maintenance/tools/remote/rdev.py -- sh -c "cargo build -p ezsdr-server && EZSDR_SERVER=/cargo-target/debug/ezsdr-server PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s python/tests"`, and under 3.9 the way gate.py's `py39` step does.
  - No test may depend on the wall clock for its verdict. A bounded wait whose expiry is the failure is allowed.
- **Do not edit**
  - handoff.md, AGENTS.md, plan/phase8/49-*.md, bench-results.md or bench/;
  - Rust code, unless a test plan item requires it. Say why if it does.
- **Reporting**
  - Do not commit, push or `git add`.
  - Name every behaviour change in the report, harmless ones included.

## Before reporting

- `cargo fmt --all --check`.
- Both Python suites.
- `rdev -- cargo test --workspace`: the docs and the Rust tests must still pass.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 8dd1016` must report 0 problems.

Report:
- `git diff --stat`;
- the client's abandon path and `close()`, as implemented;
- every test added or changed;
- the rows and their killers;
- the text changed: EA-16, the docstrings and KL-43;
- the bench tooling and its self-checks;
- every behaviour change;
- anything in the plan that proved wrong.

## Launch notes (orchestrator)

- The head is the S2 commit `486070b` (S1 `676ca5d`, start `8dd1016`).
- You run in OpenCode2 (`anthropic/claude-sonnet-5-5#high`). The self-review of AGENTS §9 step 5 is waived; a fresh Opus review of this sub-step follows your report.
- The shell tool stops a command at 120 s unless you pass a longer timeout. Give `rdev.py`, `cargo test --workspace`, the Python suites and `rules.py` an explicit timeout (e.g. 3600000 ms) and wait in the foreground until each ends. Never leave a command running in the background when you report.
- Builds and tests go through `rdev.py` on sim02/sim03; locally only edits, git (read-only), `rules.py`, `bash -n` and the driver's self-check.
- Write the full report to `/home/komatsu/.cache/ezsdr-fixes/2026-11-11-n89/report-s3.md`. Your final message is its summary.
