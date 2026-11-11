# #89 — review record and dispositions (2026-11-11)

Issue #89: a client that went away while a request blocked (`sleep`, `wait_until`, `wait_for`, `capture`, `result`) left the server transmitting (RF safety). Risk: risky (a lifecycle: the end of a Run requested from another thread). Branch `work/n89` from `8dd1016`, landing on `main` as one squashed commit.

## Process

- **Design (AGENTS §9 step 0).** Opus (effort max) drafted [n89-sketch.md](n89-sketch.md) from [n89-design-brief.md](n89-design-brief.md). The owner decided Q1–Q6 one at a time (sketch "Owner decisions" 1–10: R = Kernel signal + server input thread + Python abandon; the Simulation too; Detached refused in the stdio server; Ctrl-C and abandon both; KL-43 as a Ceiling; the bench runs). **Fable 5.2** reviewed the chosen direction ([n89-fable-design.md](n89-fable-design.md), 11 min, 218k tokens): the direction holds; F1 major (the planned tests could not kill the no-wake mutant), F2–F7 minor, A1–A3 proposals. The owner adopted A2 and F3's rule (the first end wins), deferred A1 until the E9 bench measurement and A3.
- **Test plan.** Written by the implementer (Opus subagent, effort medium) as [n89-test-plan.md](n89-test-plan.md), adjudicated in [n89-plan-adjudication.md](n89-plan-adjudication.md).
  - Sol reviewed the plan: FAIL, 2 majors and 9 minors ([n89-sol-plan.md](n89-sol-plan.md)). All adopted ([n89-plan-dispositions.md](n89-plan-dispositions.md)).
  - The Opus re-check of the amended plan found Sol's major 1 not closed (an untimed 4-zero flush already matched X1's wait) and added W5; all adopted.
- **Implementers.** S1 (Kernel, `RunHandle::disconnect_signal`, KC-36) and S2 (server: input thread, watch, A2, Detached refusal, BrokenPipe and read-error exit 0) by an Opus subagent at effort medium ([n89-brief.md](n89-brief.md)). S3 (Python client, EA-16, KL-43, bench tooling) and every later fix round by OpenCode2 `anthropic/claude-sonnet-5-5#high` (owner, 2026-11-11), self-review waived ([n89-brief-s3.md](n89-brief-s3.md), [n89-fix-briefs.md](n89-fix-briefs.md)). Each sub-step was reviewed by a fresh Opus subagent at effort high and got a full mutation run before its local commit.
- **Code review.** Sol reviewed the whole step ([n89-sol-code.md](n89-sol-code.md)): FAIL on two bench-tooling majors; no major in the Kernel, server or client.
- **Owner decisions during implementation.** PO-11 (a): the stdio server's input thread is a named exception, the governance scan forbids `thread::Builder`, and the thread-spawning tests moved to `crates/ezsdr-server/tests/`. PO-8: the server's dev-dependency on itself with the `test-support` feature (added by the implementer unasked, brought to the owner, accepted).

## Findings by source and severity

| Source | Major | Minor | Note |
|---|---|---|---|
| Fable 5.2, design | 1 (F1) | 6 (F2–F7) | A1–A3 |
| Sol, test plan | 2 | 9 | — |
| Opus, plan re-check | 1 (X1 not closed) | several (W5 added) | — |
| Opus high, S1 | 0 | 2 | 3 |
| Opus high, S2 | 1 (test reliability: X1's bound covered `connect`) | 4 | — |
| Opus high, S3 | 0 | 5 | 9 |
| Sol, code | 2 (bench E9) | 1 | — |
| Opus, re-check of Sol's fixes | 0 | 2 | 5 |

Mutation runs with failures before their fix: none beyond the known `phase7/G09`. Full runs: S1 2082/2083, S2 2099/2100 (after its fix round), S3 2114/2115.

## Dispositions after the plan

**S1 (Opus high)**
- m1, KC-36 said only a `submit` already in progress still admits and dispatches; one made after the signal does too (dispatch freezes only at cleanup's step 1). Fixed in the KC-36 text.
- m2, "in its next call" is not true of an `advance_to` to an instant already reached or a `wait_for` already matched. Fixed: "in its next call that runs a round or settles, at the latest `disconnect` or `finish`".
- Notes (K2 may not land inside KC-21a's wait; K4's holder count depends on a 1 s worker exit; an abandoned cleanup step thread also holds `Shared`): no change.

**S2 (Opus high)**
- M1, X1's 10 s socket bound also covered `connect` (up to 9.5 s on a loaded host): fixed, the bound applies after `join`.
- Minor 1, `Watched` gained a `Drop`; the orchestrator moved `Live.watched` before `run`, since fields drop in declaration order (N1 on an unwind).
- Minor 2, a failed stdin read is a departure (exit 0): named in EA-15.
- Minors 3–4: EA-15 wording; the test helpers unified.
- Rows 13, 14, 15 and 19 removed (15 and 19 equivalent under `Drop`; 13 and 14 order-only where the plan leaves the order to review); row 21 added (W6).

**S3 (Opus high: PASS with minors)**
- m1, "the server had not begun the request" as a reason for no Manifest holds only for an interrupted `connect()`: fixed in EA-16, the `close` docstring and KL-43.
- m2, kill mode could signal a reused ssh PID at exit: fixed (bounded wait, then the PID is cleared).
- m3, bench.md ran 644 `.py` files directly: fixed (`python3`).
- m4, no rows for `except BaseException` and `raise failed from None`: rows C89-3-15 and C89-3-16 added, killers checked.
- m5, kill mode could print an E9 measurement that was none (no tone before the kill; a failed kill; counted from before the stop): fixed; E9 counted from the container's `FinishedAt`.
- Notes n4–n7 adopted (EA-16's list of what interrupts a call matches the code; `bench_extract.py` classifies E9 INCONCLUSIVE lines; bench.md says how to stop a device still transmitting; the container name as the one exception to "kill by PID"). n1–n3, n8, n9 left as the review says.
- Deviations judged sound: `close()`'s own failed `finish` raises only when no Manifest is readable (as at `8dd1016`, now after the wait); `close()` after an abandoned call returns `None` (owner Q2); P5's repeated SIGINT is a race in the test (a SIGINT just before a blocking read waits for the read, INFERRED), the abandon itself pinned by P1, P2 and P6; the re-anchored phase6 P14 still killed.

**Sol, code (FAIL)**
- Major 1, an E9 measurement could follow an emergency `docker stop` that failed (the stop instant fell back to "now"): fixed; E9 is polled only after `docker inspect` reads the container stopped, else `VERDICT E9 INCONCLUSIVE`, the live-transmitter warning and a non-zero exit.
- Major 2, a tone at +1 s followed by unreadable readings printed "still on 60 s after": fixed; "still on" needs the window's last reading to be a tone, else INCONCLUSIVE with the last tone time. `ssh-drop-tx.sh --self-check` covers both with stubbed `docker`, receiver and clock.
- Minor 3, a read failing inside a frame exits 2, against EA-15's "exit 0 after a failed read": text fixed (between frames: departure, exit 0; inside a frame: answered as a frame the input cut short; the Session ends and its Manifest is written alike). No Rust change.
- Mechanism: both majors are a failure read as a value (`failure-collapsed-into-value`) in bench tooling, not in the runtime; not counted in [failure-mechanisms.md](../../maintenance/failure-mechanisms.md), which counts the runtime's design mechanisms.

**Opus re-check of Sol's fixes**
- Both majors and the minor closed; the moved script functions are byte-identical except the two fixed ones.
- Minor A, a quiet first reading after the death printed as a bracketed measurement: now an upper-bound line (`ended by +N ms`), still a measurement for A1.
- Minor B, unreadable readings inside the bracket kept the fixed "resolution 1 s": the line prints the bracket's width.
- EA-15's unsupported "status 2" dropped. Notes 1–4 left.

## Gate

- First run (`6c96831`): FAIL, five s15 paired parity tests failed together in the workspace test step. Not #89's: no code they run changed; the same tree then passed 6 of 6 workspace runs and the parity binary 9 of 9, alternating with `8dd1016` on sim02 and sim03 (all passed); the failures coincided with load on sim02. Filed as #96 (`mech:wall-clock-test-oracle`).
- Second run: PASS ([n89-gate.json](n89-gate.json)): 2267 tests, 532 UHD, Python 121 × 2, mutation 2114/2115 (`phase7/G09` known).

## Not done here

- The bench after the fix (owner, Q6): `sleep` at `8dd1016` (FAIL expected), `sleep` at the new commit (PASS expected), and `kill` measuring E9; then A1. Procedure: [bench.md](../bench.md) "#89 の後".
- #89 has no `mech:` label: the defect was a missing path (no departure seen during a blocking request), not one of the listed mechanisms.
