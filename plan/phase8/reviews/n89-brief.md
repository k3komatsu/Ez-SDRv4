# Brief — #89: a client that goes away ends its Session, also while a request waits

Repository /home/komatsu/local_work/Ez-SDRv4, branch `work/n89` from `main` `8dd1016`. Read first:
- AGENTS.md "Safety core", §3, §6, §7 and §9 (you are the implementer);
- issue #89.

**The design** (AGENTS §9 step 0, done):
- `/home/komatsu/.cache/ezsdr-fixes/2026-11-11-n89/sketch.md`, an Opus draft. Its first section, **"Owner decisions" 1–10**, overrides the rest.
- The design review `/home/komatsu/.cache/ezsdr-fixes/2026-11-11-n89/fable-design.md` (Fable 5.2). Its F1–F7, N1–N5 and A2 are adopted; A1 and A3 are deferred.

Read both in full. Where you find either one wrong against the code, say so with evidence; do not follow it blindly.

**Risk: risky** (how a Run ends, requested from another thread; KC-36; the user-facing meaning of EA-15 and EA-16). So write **a test plan first, no code**; Sol reviews it; then you implement.

## Scope (sketch §2, as amended by the decisions and the review)

- **S1, Kernel.**
  - Add `RunHandle::disconnect_signal()` (sketch §2.1). Account for:
    - F2: a `submit` that is already read can still dispatch, so the KC-36 text must not claim otherwise;
    - N1: the closure upgrades a `Weak`, and the device must not drop on the input thread;
    - N2: wake in "a class that waits in host time", not only `device_paced()`.
  - KC-36 and KC-46a text. No new Kernel type unless review asks.
- **S2, Server.**
  - `serve_watched` with an input thread over an unbounded channel (N3).
  - The watch, holding the Session's signal and a child's (A3 deferred), with N1's mutex discipline.
  - **A2**: once the input has ended, the server serves no further request.
  - Refuse a Detached Lease at `connect` (Q3).
  - `main.rs`: the watched input, and a `BrokenPipe` reply write ends with exit 0 and no message.
  - EA-10 and EA-15 text, with F5's exit status and A2's meaning.
- **S3, Python and tooling.**
  - `start_new_session=True`.
  - Abandon on any exception while a reply is awaited (F6): the `try` covers the write too; close both pipes; `close()` is repeatable after an abandon; no `finish`; wait for the exit, then read `manifest.json`, tolerating one that is missing or partial.
  - Fix the order in the existing fallback: wait for the exit, then read.
  - EA-16 text and the docstrings.
  - **KL-43**: silent ssh partitions; signals to, or death of, the server itself; F4, a hung server no longer killed by Ctrl-C; F5's three additions. The sshd workaround is written generically: any host that runs the server over ssh.
  - `ssh-drop-driver.py` and `ssh-drop-tx.sh`: `EZSDR_DROP_WAIT=time|sleep`.
  - `plan/phase8/bench.md`: a new procedure section for the #89 bench. It has three runs: (i) with `sleep` at `8dd1016` (FAIL expected), (i) with `sleep` at the new commit (PASS), and a `docker kill` of the server during a repeat with #2 listening, to measure E9. Include F7's hold margin.
- **F3**: in the contract, `finish` overlapping a departure is "the first end requested wins" (KC-32).

Evidence to cite, without re-deriving it:
- E5: `Provider::stop` took about 104 ms on X300 #1 in the 2026-11-09 (i) run. From the raw archive's Manifest `timing` row, at 1479191981, done 1500026409 root ticks at 200 MHz. VERIFIED, one sample.
- The rest is in sketch §7.

## Operation sequences

Use sketch §10's sequences 1–13 as amended:
- Sequence 5 becomes "the first end wins".
- Add A2: input ends with a frame already read → that frame is not served.
- Add the F1 requirement: the Kernel wake test observes the control thread parked in `next_wakeup` (a probe on the authority) **before** it fires. Without that, the "no wake" mutant survives, and that mutant is #89 itself.
- Add F6's Python cases: an exception during the write, a repeated `close()`, a missing Manifest.

For each sequence give its sub-step, the expected result and what it relies on (VERIFIED or INFERRED). No test may depend on the wall clock for its verdict; a bounded wait whose expiry is the failure is fine, as in the existing tests.

## Test plan (deliver this first, then STOP and report)

Write `/home/komatsu/.cache/ezsdr-fixes/2026-11-11-n89/test-plan.md`, organised by sub-step. For each sequence give:
- the test, existing to change or new (name, file);
- what it asserts;
- how it observes the behaviour;
- the wrong behaviour that would fail it.

The plan also includes:
- the mutation rows to add, retarget or re-anchor (`plan/maintenance/tools/mutations.json`, Rust token anchors). Each row names its killing test in `filter`, and the plan shows that the killer runs the mutated path.
- the existing tests that change, with the reason.
- the Python suites: which tests are added and how they run (`python -m unittest discover -s python/tests`, as gate.py does).
- the hardware audit (AGENTS §9 "Hardware tests and bench values"): which `#[ignore]` tests or bench tools depend on the changed behaviour, and their FakeDevice or subprocess rehearsal.

Do not write code until the orchestrator says the plan is accepted.

## Rules (binding)

- Do not commit, push or `git add`. Do not run `mutate.py` and do not apply a mutant inside the tree. A /tmp copy outside the repository is fine for checking a killer.
- Builds and tests: locally with a target outside the tree, or `python3 plan/maintenance/tools/remote/rdev.py -- <cargo …>`, in the foreground until it ends. Python runs with `PYTHONDONTWRITEBYTECODE=1`.
- Before reporting code:
  - run `cargo fmt --all`;
  - run `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 8dd1016 [--renames <file>]` and get 0 problems.
- Do not edit `handoff.md`, `AGENTS.md`, `plan/phase8/49-*.md`, `plan/phase8/bench-results.md` or `bench/`.
- Name every behaviour change in your report, harmless ones included.
