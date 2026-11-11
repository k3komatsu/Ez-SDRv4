# Design brief — #89: a client that goes away while a request blocks must stop the transmit (AGENTS §9 step 0)

Repository: /home/komatsu/local_work/Ez-SDRv4 on `main` at `8dd1016` (a clean tree). You are the **design drafter** (AGENTS.md §9 step 0, Opus at effort max). You are read-only: do not edit anything in the tree. Write your draft to `/home/komatsu/.cache/ezsdr-fixes/2026-11-11-n89/sketch.md` and report a short summary.

## The problem (issue #89, `gh issue view 89 -R k3komatsu/Ez-SDRv4`; read it in full)

EA-15 (`design/16-easy-api.md:81`) promises that when the server's input ends before `finish`, the server calls `disconnect()`. That holds only between requests.
- `crates/ezsdr-server/src/transport.rs:24-83` reads a frame, calls `server.handle`, then writes the reply. It does not read stdin while a request runs.
- `advance` and `wait_for` block synchronously on the RunHandle (`crates/ezsdr-server/src/lib.rs:485-513`). On device-paced Runs they wait real time (`crates/ezsdr-kernel/src/coordinator/stepping.rs:311-358`).
- The waits the docs recommend are each one blocking request: `Session.sleep`, `capture`/`result`, and `wait` (`python/ezsdr/session.py:433-456, 695-698`).

This gives three sequences:
- **(a) ssh drop during `sdr.sleep(600)`.** The server keeps transmitting until the request ends.
- **(b) Ctrl-C in a terminal.** SIGINT reaches the server too (`python/ezsdr/_client.py:67` starts it without `start_new_session`; the server has no signal handler). The server dies at once: no `Provider::stop`, no EOB close, no Manifest.
- **(c) SIGINT to Python only.** `close()` sends `finish` and reads the stale `advanced` reply: `KeyError: 'manifest'`.

Earlier evidence: bench (i) `plan/phase8/tools/ssh-drop-tx.sh` + `ssh-drop-driver.py` passed on 2026-11-09 with `time.sleep(hold)`, that is, with an idle server (bench-results.md "2026-11-09" (i)). The Fable 5.2 repository audit of 2026-11-11 found the gap (`/home/komatsu/.cache/ezsdr-fixes/2026-11-11-fable-audit/report.md`, F1).

## Goal

Whatever way the client goes away or is interrupted, while the server is idle or inside any request, the Session ends the way EA-15 promises:
- within a bounded time;
- through `disconnect()`, so `Provider::stop` runs, the device burst closes and the Manifest is written with its stop reason;
- with the client side giving the user a clear outcome: no stale-reply confusion, and a Manifest path when one exists.

The RF safety envelope matters most here: transmit must not outlive its client by more than the bound.

## Constraints already decided (do not relitigate)

- AGENTS §3: the Kernel stays small, and the real-time path never parses JSON. Everything is a Run, and an Attached Lease ends with its client (EA-15, Vision §50–§52).
- Pre-release: breaking the Python API or the protocol is allowed when it is better long-term (AGENTS §6). There is no version management.
- Proportionate (owner): the lean fix first, and refusing or deleting is preferred over frameworks.
- **Rejected by the audit, with reasons. You may challenge them with evidence.**
  - Slicing `sleep` only in Python: it cannot protect an ssh drop.
  - A TTL on Attached Leases: it changes the Lease's meaning.

## What the draft must contain (AGENTS §9 step 0)

1. **The user-facing contract.** What happens on each way of leaving: ssh or pipe EOF, SIGINT to the process group, SIGINT to Python only, SIGTERM, SIGKILL of the client, SIGKILL of the server, and a Python exception inside a `with` block. Cover each state: idle, inside `advance`/`wait_for`/`sleep`/`capture`/`result`, inside `connect`, and inside `finish`. For each, give the bound, what the user sees, and what the Manifest says.
2. **At least two alternatives, with the reasons each is rejected.** Candidates, which you need not be limited to:
   - a stdin reader thread plus sliced device-paced waits;
   - a cancellation token or condition the RunHandle wait observes;
   - signal handling in the server (SIGINT ignored or turned into disconnect; SIGTERM);
   - the Python side: `start_new_session`, `KeyboardInterrupt` handling, closing stdin instead of sending `finish`;
   - a heartbeat in the protocol.
   For each: what it covers among (a)–(c), where it lives (server, Kernel or Python), and its cost.
3. **The principles that decide the cases.** For example: who owns the end of the Session; whether a signal is the same as EOF; whether a simulation Run, which is not device-paced, should behave differently.
4. **A check against the existing promises.** EA-2, EA-15 and the other EA rules in `design/16-easy-api.md`; the Kernel's RunHandle and stop semantics (the RS rules in `design/`, KC-21a); the UHD Module's `Provider::stop` and cleanup bounds (UR rules in `design/18-uhd-radio.md`, RS-8a 5 s); `KNOWN_LIMITATIONS.md`. List every rule text that would change.
5. **Evidence.** The bench and UHD facts the design relies on, marked VERIFIED or INFERRED: how long `Provider::stop` takes on the X300 and whether the device keeps transmitting after the process dies (see bench-results.md for "kill", "SIGKILL", "ssh 切断", "(i)", "C1", "C2"). Whether the X300 stops a repeat by itself when the host dies uncleanly (a repeat is host-fed, so starvation: INFERRED unless evidence exists).
6. **What the choice makes impossible later.** For example, a Detached Lease (does one exist? check), or multiple clients.
7. **The test approach.** How each sequence is tested without hardware (FakeDevice, a fake RunHandle, a subprocess with closed pipes, signals in tests without wall-clock flakiness), and the hardware check: `ssh-drop-driver.py` with an `sdr.sleep(hold)` variant, FAIL at the old commit and PASS at the new (owner, 2026-11-11). Say which existing tests change.
8. **The risk class and the staging**, if the change is large (AGENTS §9 step 1 "risky").

Lead with one recommendation and the rejected alternatives (AGENTS §6). Cite `path:line` for every claim about the code. Mark what you could not verify as INFERRED. Write in English or Japanese with English technical terms; your choice.
