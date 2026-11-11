# #89 — fix-round briefs (S3 and after Sol's code review)


## Brief — #89 S3 fix round

Same tree and branch as before (`work/n89`, HEAD `486070b`; S3 is the uncommitted diff plus `plan/phase8/tools/ssh-drop-relay.py`). Read, in /home/komatsu/.cache/ezsdr-fixes/2026-11-11-n89/: brief-s3.md (its rules and prohibitions still bind, including the launch notes at its end), your report report-s3.md, and the review review-s3.md. Then fix the adopted findings below. Nothing else changes.

## Adopted (orchestrator 2026-11-11)

- **m1** EA-16 (`design/16-easy-api.md` ~:127), the `close` docstring and KL-43 (e): "the server had not begun the request" as a reason for no Manifest applies only to an interrupted `connect()`. Say so; once a Session exists, every exit writes the Manifest.
- **m2** `ssh-drop-tx.sh` kill mode: after ssh exits, wait for it with a bounded `kill -0` loop / `wait`, then clear `SSH_PID` so the exit cleanup never signals a reused PID.
- **m3** `bench.md`: every command running the driver or the relay uses `python3 <file>.py` (the files are 644 with no shebang).
- **m4** Add rows C89-3-15 (`except BaseException` → `except Exception` at `_client.py`, killer P1 or whichever test you verify) and C89-3-16 (`raise failed from None` in `session.py`, killer P3d). Check each killer in a /tmp copy outside the repository, as before.
- **m5** Kill-mode E9:
  - with no tone seen before the kill, print `VERDICT E9 INCONCLUSIVE: no tone before the kill` (as C2 does in link mode) and exit non-zero;
  - if `docker kill` fails, print an INCONCLUSIVE line naming that, never a measurement, and exit non-zero;
  - the E9 taken after the emergency stop: count from the moment the container is known to have stopped (e.g. after `docker stop` returns, or the time `docker inspect` reports for `State.FinishedAt`), not from before `docker stop -t 2`; say in bench.md which instant it is and its error bound.
- **n4** EA-16's list of what interrupts a call: make it match the code (a header that is not JSON or lacks `reply` abandons; a `reply` with neither `result` nor `error` raises outside the abandon and abandons nothing). Text only; do not change the code.
- **n5** `plan/phase8/tools/bench_extract.py`: classify `VERDICT E9 INCONCLUSIVE: …` lines with verdict `INCONCLUSIVE` (measurement lines stay without a verdict). If the tool has a self-check or tests, extend them.
- **n6** `bench.md` (ii) and (i-a): when the tone is still on 60 s after the server's death, say what to do: tell the owner; stop the transmitter by power-cycling the USRP (or a fresh UHD session only on the owner's word).
- **n7** `bench.md`: half a line saying `docker kill`/`docker stop` on the exact container name is the one exception to "kill by PID", as with the existing emergency stop.

Not adopted: n1, n2, n3, n8, n9 (acceptable as the review says).

## Before reporting

As brief-s3 says: `cargo fmt --all --check` (no Rust should change), both Python suites (3.14 via rdev, 3.9 as gate.py's py39 step), `bash -n`, the self-checks, `rules.py --base 8dd1016` 0 problems. Run `cargo test --workspace` only if you touched Rust (you should not).

Append a "Fix round" section to report-s3.md: each item, what changed (file:line), the new rows and their verified killers. Your final message is that section's summary.

## Brief — #89 fix round after Sol's code review

Tree: /home/komatsu/local_work/Ez-SDRv4, branch `work/n89`, HEAD `252f4ad` (S1 676ca5d, S2 486070b, S3 252f4ad; start 8dd1016). Ignore the uncommitted handoff.md hunk (the orchestrator's). Read, in /home/komatsu/.cache/ezsdr-fixes/2026-11-11-n89/: brief-s3.md (its rules, prohibitions and launch notes still bind), report-s3.md (with its fix-round section), and Sol's review sol-code.md. Then fix the adopted findings below and nothing else.

## Adopted (orchestrator 2026-11-11)

1. **Major 1 — E9 after a death that was not confirmed** (`plan/phase8/tools/ssh-drop-tx.sh`, `stopped_ns` ~:259–271, the emergency stop ~:428–434, the kill path ~:410–420).
   - `stopped_ns` gives an instant only when `docker inspect` reads `running=false` (FinishedAt when it parses, else the return of the stopping command — but only once the container is confirmed stopped). When the container is still running or cannot be inspected, it gives no instant.
   - Both E9 paths (kill mode, and the emergency stop in link mode) poll E9 only after a confirmed stop. Otherwise print `VERDICT E9 INCONCLUSIVE: <the container was not confirmed stopped: state …>; this is no measurement`, keep the live-server warning (the device may still be transmitting: tell the owner), and exit non-zero.
   - A failed `docker stop` in the emergency path is checked, not ignored.
2. **Major 2 — an early tone and an unreadable tail read as "still on 60 s after"** (`e9_poll` ~:232–254).
   - "still on 60 s after" only when the last reading of the 60 s window read the tone (the reading at or just before the deadline, within one poll interval of it).
   - When a tone was read and every later reading up to the deadline was unreadable, print `VERDICT E9 INCONCLUSIVE: the tone was last read at +N ms and the receiver was unreadable after it` (machine-printed values), not a measurement, and exit non-zero.
   - Likewise an unreadable reading must never count as quiet (check that it does not).
3. **Tests for both, device-free.** Give `ssh-drop-tx.sh` a `--self-check` (as the driver and the relay have) that sources the script's functions with stubbed `docker` and receiver readings and asserts: running=true → no instant and INCONCLUSIVE; inspect failure → INCONCLUSIVE; running=false with FinishedAt → that instant; tone then unreadable to the deadline → INCONCLUSIVE with the last tone time; tone then quiet → measurement; tone through the last reading → "still on". It must run without hardware, docker or ssh, finish in seconds, and need no wall-clock verdict (stub the clock/readings). Keep it small; restructure the script only as much as sourcing the functions needs. Add it to bench.md's pre-bench checks.
4. **bench.md**: the E9 guarantees at ~:1166–1168, ~:1182, ~:1209 claim too much; make them state exactly what the script now guarantees (a measurement only after a confirmed stop and with a reading on each side of the tone's end; INCONCLUSIVE otherwise).
5. **Minor 3 — a failed read inside a frame exits 2** (text only; do not change Rust). A read error after a partial header or body is answered as a stream that ended there (EA-2, `BadFrame`, exit 2); the Session still ends and the Manifest is written. Make EA-15 (`design/16-easy-api.md` ~:83, "as it does after a failed read") and any other user text that promises exit 0 for every failed read (KL-43, docstrings, bench.md) say: a read that fails between frames exits 0; one that fails inside a frame is a stream that ended inside it (exit 2), with the Session ended and its Manifest written alike. Keep the design/ rule format (no issue numbers in the rule text).

## Before reporting

`bash -n`, the new `--self-check` and the existing driver/relay self-checks, `bench_extract.py`'s self-check if it has one, `rules.py --base 8dd1016` 0 problems, `cargo fmt --all --check` (no Rust should change). Python suites only if Python changed (it should not). If a mutation row's anchor text changed, re-anchor it and verify its killer in a /tmp copy outside the repository; add no shell mutation rows unless one is cheap and its killer is the new self-check.

Append a section "Fix round after Sol" to report-s3.md: each item, what changed (file:line), the self-check's cases, every behaviour change. Your final message is that section's summary.

## Brief — #89 small fix after the re-check

Tree as before (`work/n89`, HEAD `252f4ad`, the post-Sol fix uncommitted). Read, in /home/komatsu/.cache/ezsdr-fixes/2026-11-11-n89/: brief-s3.md (rules, prohibitions, launch notes bind), brief-sol-fix.md, and the re-check recheck-sol-fix.md. Fix only these (minor; orchestrator adopted 2026-11-11):

- **A. Quiet on the first reading after the death** (`plan/phase8/tools/ssh-drop-tx.sh`, `e9_poll`). No tone was read after the death, so the line is an upper bound only. Print it with a distinct, machine-printed form, e.g. `E9 ended by +N ms (upper bound: no tone was read after <event>; the last tone was read before it)`, distinct from the bracketed `ended after` line. It still counts as measured (exit 0 allowed): an upper bound is what the A1 decision needs. Add a self-check case.
- **B. Unreadable readings inside the bracket.** The bracketed line prints the bracket's actual width (first quiet time − last tone time) instead of the fixed "(the monitor's resolution is 1 s)". Add a self-check case (tone at +500 ms, unreadable to +39.5 s, quiet at +40 s → width 39.5 s).
- **bench.md** "E9 の保証" and the reading instructions: say exactly this — a bracketed line (last tone, first quiet, its width as the error) or an upper-bound line; everything else INCONCLUSIVE.
- **EA-15** (`design/16-easy-api.md`, the paragraph added in the last round): "status 2 as for any frame the input cuts short" — status 2 is stated nowhere else in spec 16. Say only that it is answered as a frame the input cut short (EA-2/EA-5); drop the claimed status unless you cite where the binary's exit codes are specified. Keep the rule format.

Not adopted: notes 1–4 of the re-check.

Before reporting: `bash -n`, `--self-check` (all cases pass), check in a /tmp copy that each new case fails on the old behaviour, `rules.py --base 8dd1016` 0 problems. Append "Fix round after the re-check" to report-s3.md; your final message is its summary.
